use std::{
    cell::UnsafeCell,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use crate::{Stack, State};

pin_project_lite::pin_project! {
    pub struct CtxFuture<'a, 's, F, R> {
        ptr: NonNull<Stack>,
        // The function to execute to get the future
        f: Option<F>,
        // The place where the future will store the result.
        res: UnsafeCell<Option<R>>,
        // The future holds onto the ctx mutably to prevent another future being pushed before the
        // current is polled.
        _marker: PhantomData<&'a mut Stk<'s>>,
    }
}

impl<'a, 's, F, Fut, R> Future for CtxFuture<'a, 's, F, R>
where
    F: FnOnce(Stk<'s>) -> Fut,
    Fut: Future<Output = R>,
{
    type Output = R;

    #[inline]
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        unsafe {
            if let Some(x) = this.f.take() {
                let ctx = Stk::new_ptr(*this.ptr);

                this.ptr.as_ref().tasks.push(TaskFuture {
                    place: NonNull::from(this.res),
                    inner: (x)(ctx),
                });

                this.ptr.as_ref().state.set(State::NewTask);
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
            unsafe {
                ptr.as_ref().state.set(State::Yield);
            }
            return Poll::Pending;
        }
        Poll::Ready(())
    }
}

pub struct Stk<'s> {
    ptr: NonNull<Stack>,
    _marker: PhantomData<&'s Stack>,
}

impl<'s> Stk<'s> {
    pub(super) unsafe fn new_ptr(ptr: NonNull<Stack>) -> Self {
        Stk {
            ptr,
            _marker: PhantomData,
        }
    }
}

unsafe impl<'s> Send for Stk<'s> {}
unsafe impl<'s> Sync for Stk<'s> {}

impl<'s> Stk<'s> {
    /// Run a new future in the runtime.
    pub fn run<'a, F, Fut, R>(&'a mut self, f: F) -> CtxFuture<'a, 's, F, R>
    where
        F: FnOnce(Stk<'s>) -> Fut,
        Fut: Future<Output = R>,
    {
        CtxFuture {
            ptr: self.ptr,
            f: Some(f),
            res: UnsafeCell::new(None),
            _marker: PhantomData,
        }
    }

    /// Wrap a function, allowing turning a `&mut Ctx` into a Ctx<'_>
    ///
    /// This function runs the returned future immediatly, it does not(!) have any guarentees about
    /// not increasing stack usages.
    pub async fn wrap<'a, F, Fut, R>(&'a mut self, f: F) -> R
    where
        F: FnOnce(Stk<'s>) -> Fut,
        Fut: Future<Output = R>,
    {
        let new_ctx = unsafe { Stk::new_ptr(self.ptr) };
        f(new_ctx).await
    }

    pub fn reborrow<'a>(&'a mut self) -> Stk<'a> {
        Stk {
            ptr: self.ptr,
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
