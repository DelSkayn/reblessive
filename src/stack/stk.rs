use crate::stack::{with_stack_context, Stack, State};
use pin_project_lite::pin_project;
use std::{
    cell::UnsafeCell,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

/// Future returned by [`Stk::run`]
///
/// Should be immediatly polled when created and driven until finished.
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct StkFuture<'a, F, R> {
    // The function to execute to get the future
    f: Option<F>,
    completed: bool,
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
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Pinning isn't structural for any of the fields.
        let this = unsafe { self.get_unchecked_mut() };
        unsafe {
            if let Some(x) = this.f.take() {
                with_stack_context(|stack| {
                    let place = NonNull::from(&this.res);
                    let fut = (x)(Stk::new());

                    stack
                        .tasks
                        .push(async move { place.as_ref().get().write(Some(fut.await)) });
                    stack.set_state(State::NewTask);
                });
                return Poll::Pending;
            }

            if let Some(x) = (*this.res.get()).take() {
                // Set the this pointer to null to signal the drop impl that we don't need to pop
                // the task.
                this.completed = true;
                return Poll::Ready(x);
            }
        }
        Poll::Pending
    }
}

impl<'a, F, R> Drop for StkFuture<'a, F, R> {
    fn drop(&mut self) {
        if self.f.is_none() && !self.completed && self.res.get_mut().is_none() {
            // F is none so we did push a task but we didn't yet return the value and it also isn't
            // in its place. Therefore the task is still on the stack and needs to be popped.
            with_stack_context(|stack| {
                unsafe { stack.tasks().pop() };
            })
        }
    }
}

pin_project! {
    /// Future returned by [`Stk::yield_now`]
    #[must_use = "futures do nothing unless you `.await` or poll them"]
    pub struct YieldFuture{
        pub(crate) done: bool,
    }
}

impl Future for YieldFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        with_stack_context(|stack| {
            if !*this.done {
                *this.done = true;
                stack.set_state(State::Yield);
                return Poll::Pending;
            }
            Poll::Ready(())
        })
    }
}

/// A reference back to stack from inside the running future.
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
            completed: false,
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
