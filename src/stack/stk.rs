use std::{
    cell::UnsafeCell,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

use pin_project_lite::pin_project;

use super::State;
use crate::{stub_ctx::WakerCtx, Stack};

/// Future returned by [`Stk::run`]
///
/// Should be immediatly polled when created and driven until finished.
pub struct StkFuture<'a, F, R> {
    // A pointer to the stack, created once future task is pushed.
    ptr: Option<NonNull<Stack>>,
    // The function to execute to get the future
    f: Option<F>,
    // The place where the future will store the result.
    res: UnsafeCell<Option<R>>,
    // The future holds onto the ctx mutably to prevent another future being pushed before the
    // current is polled.
    _marker: PhantomData<&'a mut Stk>,
}

impl<'a, F, Fut, R> Future for StkFuture<'a, F, R>
where
    F: FnOnce(&'a mut Stk) -> Fut,
    Fut: Future<Output = R> + 'a,
{
    type Output = R;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Pinning isn't structural for any of the fields.
        let this = unsafe { self.get_unchecked_mut() };
        unsafe {
            if let Some(x) = this.f.take() {
                let stk = WakerCtx::<Stack>::ptr_from_waker(cx.waker());
                let stk_ptr = NonNull::from(stk.as_ref().stack);
                this.ptr = Some(stk_ptr);

                let ctx = Stk::new();

                stk_ptr.as_ref().tasks.push(TaskFuture {
                    place: NonNull::from(&this.res),
                    inner: (x)(ctx),
                });

                stk_ptr.as_ref().state.set(State::NewTask);
                return Poll::Pending;
            }

            if let Some(x) = (*this.res.get()).take() {
                // Set the this pointer to null to signal the drop impl that we don't need to pop
                // the task.
                this.ptr = None;
                return Poll::Ready(x);
            }
        }
        Poll::Pending
    }
}

impl<'a, F, R> Drop for StkFuture<'a, F, R> {
    fn drop(&mut self) {
        if let Some(ptr) = self.ptr {
            unsafe {
                if (*self.res.get()).is_none() {
                    // it ptr is some but self.res is none then the task was pushed but has not yet
                    // completed and must be dropped first.
                    ptr.as_ref().tasks.pop();
                }
            }
        }
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
    /// Future returned by [`Stk::yield_now`]
    pub struct YieldFuture{
        done: bool,
    }
}

impl Future for YieldFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        let stk = WakerCtx::<Stack>::ptr_from_waker(cx.waker());

        if !*this.done {
            *this.done = true;
            unsafe {
                stk.as_ref().stack.state.set(State::Yield);
            }
            return Poll::Pending;
        }
        Poll::Ready(())
    }
}

/// A refernce back to stack from inside the running future.
///
/// Used for spawning new futures onto the stack from a future running on the stack.
pub struct Stk(PhantomData<*mut Stack>);

impl Stk {
    pub(super) unsafe fn new() -> &'static mut Self {
        NonNull::dangling().as_mut()
    }
}

impl Stk {
    /// Run a new future in the runtime.
    pub fn run<'a, F, Fut, R>(&'a mut self, f: F) -> StkFuture<'a, F, R>
    where
        F: FnOnce(&'a mut Stk) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        StkFuture {
            ptr: None,
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
        YieldFuture { done: false }
    }
}
