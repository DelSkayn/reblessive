use futures_util::future::poll_fn;

use super::{with_tree_context, TreeStack};
use crate::{
    stack::{enter_stack_context, with_stack_context, State, YieldFuture},
    Stack,
};
use std::{
    cell::UnsafeCell,
    future::Future,
    marker::PhantomData,
    pin::{pin, Pin},
    ptr::NonNull,
    sync::Arc,
    task::{ready, Context, Poll},
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
                        .tasks()
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

pub struct ScopeFuture<'a, F, R> {
    entry: Option<F>,
    place: Arc<UnsafeCell<Option<R>>>,
    _marker: PhantomData<&'a Stk>,
}

impl<'a, F, R> ScopeFuture<'a, F, R> {
    pin_utils::unsafe_unpinned!(entry: Option<F>);
}

impl<'a, F, Fut, R> Future for ScopeFuture<'a, F, R>
where
    F: FnOnce(&'a ScopeStk) -> Fut,
    Fut: Future<Output = R> + 'a,
    R: 'a,
{
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        if let Some(x) = this.entry.take() {
            with_tree_context(|fanout| {
                let fut = unsafe { (x)(ScopeStk::new()) };
                let place = this.place.clone();
                let waker = cx.waker().clone();
                unsafe {
                    fanout.push(async move {
                        place.get().write(Some(fut.await));
                        waker.wake();
                    })
                }
            });
            Poll::Pending
        } else {
            if let Some(x) = unsafe { std::ptr::replace(this.place.get(), None) } {
                Poll::Ready(x)
            } else {
                Poll::Pending
            }
        }
    }
}

/// A refernce back to stack from inside the running future.
///
/// Used for spawning new futures onto the stack from a future running on the stack.
pub struct Stk(PhantomData<*mut TreeStack>);

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
            f: Some(f),
            completed: false,
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

    pub fn scope<'a, F, Fut, R>(&'a mut self, f: F) -> ScopeFuture<'a, F, R>
    where
        F: FnOnce(&'a ScopeStk) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        ScopeFuture {
            entry: Some(f),
            place: Arc::new(UnsafeCell::new(None)),
            _marker: PhantomData,
        }
    }
}

#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct ScopeStkFuture<'a, R> {
    place: NonNull<UnsafeCell<Option<R>>>,
    stack: Stack,
    _marker: PhantomData<&'a ScopeStk>,
}

impl<'a, R> Drop for ScopeStkFuture<'a, R> {
    fn drop(&mut self) {
        unsafe { std::mem::drop(Box::from_raw(self.place.as_ptr())) };
    }
}

impl<'a, R> ScopeStkFuture<'a, R> {
    pin_utils::unsafe_unpinned!(stack: Stack);
    pin_utils::unsafe_unpinned!(place: NonNull<UnsafeCell<Option<R>>>);
}

impl<'a, R> Future for ScopeStkFuture<'a, R> {
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        enter_stack_context(&this.stack, || loop {
            match this.stack.drive_head(cx) {
                Poll::Pending => {
                    return Poll::Pending;
                }
                Poll::Ready(_) => match this.stack.get_state() {
                    State::Base => {
                        if let Some(x) = unsafe { (*this.place.as_ref().get()).take() } {
                            return Poll::Ready(x);
                        }
                        return Poll::Pending;
                    }
                    State::NewTask => this.stack.set_state(State::Base),
                    State::Yield => todo!(),
                    State::Cancelled => return Poll::Pending,
                },
            }
        })
    }
}

/// A refernce back to stack from inside the running future.
///
/// Used for spawning new futures onto the stack from a future running on the stack.
pub struct ScopeStk {
    marker: PhantomData<*mut TreeStack>,
}

impl ScopeStk {
    pub(super) unsafe fn new() -> &'static mut Self {
        NonNull::dangling().as_mut()
    }
}

impl ScopeStk {
    /// Run a new future in the runtime.
    pub fn run<'a, F, Fut, R>(&'a self, f: F) -> ScopeStkFuture<'a, R>
    where
        F: FnOnce(&'a mut Stk) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        let stack = Stack::new();
        let place = Box::into_raw(Box::new(UnsafeCell::new(None)));
        let place = unsafe { NonNull::new_unchecked(place) };
        let future = unsafe { f(Stk::new()) };

        stack.tasks().push(async move {
            let mut pin_future = pin!(future);
            poll_fn(move |cx: &mut Context| -> Poll<()> {
                let res = ready!(pin_future.as_mut().poll(cx));
                unsafe { place.as_ref().get().write(Some(res)) };
                Poll::Ready(())
            })
            .await
        });

        ScopeStkFuture {
            place,
            stack,
            _marker: PhantomData,
        }
    }
}
