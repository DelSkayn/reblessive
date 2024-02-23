use std::{
    cell::UnsafeCell,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use crate::Stack;

pin_project_lite::pin_project! {
    pub struct CtxFuture<'a, 's, F, R> {
        ptr: NonNull<Stack>,
        f: Option<F>,
        res: UnsafeCell<Option<R>>,
        _marker: PhantomData<&'a mut Ctx<'s>>,
    }
}

impl<'a, 's, F, Fut, R> Future for CtxFuture<'a, 's, F, R>
where
    F: FnOnce(Ctx<'s>) -> Fut,
    Fut: Future<Output = R>,
{
    type Output = R;

    #[inline]
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        unsafe {
            if let Some(x) = this.f.take() {
                let ctx = Ctx::new_ptr(*this.ptr);

                this.ptr.as_ref().0.push(TaskFuture {
                    place: NonNull::from(this.res),
                    inner: (x)(ctx),
                });

                return Poll::Pending;
            }

            if let Some(x) = (*this.res.get()).take() {
                return Poll::Ready(x);
            }
        }
        Poll::Pending
    }
}

pin_project! {
    pub(super) struct TaskFuture<F, R> {
        pub place: NonNull<UnsafeCell<Option<R>>>,
        #[pin]
        pub inner: F,
    }
}

impl<F, R> Future for TaskFuture<F, R>
where
    F: Future<Output = R>,
{
    type Output = ();

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        unsafe {
            let res = std::task::ready!(this.inner.poll(cx));
            this.place.as_ref().get().write(Some(res));
            Poll::Ready(())
        }
    }
}

pin_project! {
    pub struct YieldFuture{
        ptr: Option<NonNull<Stack>>,
    }
}

impl Future for YieldFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        if let Some(ptr) = this.ptr.take() {
            // TODO: The current executor figures out if it needs to continue running tasks by if a
            // new task was scheduled. So we need to schedule an empty task to
            unsafe {
                ptr.as_ref().0.push(async {});
            }
            return Poll::Pending;
        }
        Poll::Ready(())
    }
}

pub struct Ctx<'s> {
    ptr: NonNull<Stack>,
    _marker: PhantomData<&'s Stack>,
}

impl<'s> Ctx<'s> {
    pub(super) unsafe fn new_ptr(ptr: NonNull<Stack>) -> Self {
        Ctx {
            ptr,
            _marker: PhantomData,
        }
    }
}

unsafe impl<'s> Send for Ctx<'s> {}
unsafe impl<'s> Sync for Ctx<'s> {}

impl<'s> Ctx<'s> {
    /// Run a new future in the runtime.
    pub fn run<'a, F, Fut, R>(&'a mut self, f: F) -> CtxFuture<'a, 's, F, R>
    where
        F: FnOnce(Ctx<'s>) -> Fut,
        Fut: Future<Output = R>,
    {
        CtxFuture {
            ptr: self.ptr,
            f: Some(f),
            res: UnsafeCell::new(None),
            _marker: PhantomData,
        }
    }

    /// Yield the execution of the recursive futures back to the reblessive runtime.
    ///
    /// When stepping through a function instead of finishing it awaiting the future returned by
    /// this function will cause the the current step to complete.
    pub fn yield_now(&mut self) -> YieldFuture {
        YieldFuture {
            ptr: Some(self.ptr),
        }
    }
}
