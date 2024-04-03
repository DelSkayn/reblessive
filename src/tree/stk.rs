use pin_utils::{unsafe_pinned, unsafe_unpinned};

use super::{with_tree_context, TreeStack};
use crate::{
    stack::{
        enter_stack_context, with_stack_context, InnerStkFuture, StackMarker, State, YieldFuture,
    },
    Stack,
};
use std::{
    cell::UnsafeCell,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    ptr::NonNull,
    sync::Arc,
    task::{Context, Poll},
};

impl StackMarker for Stk {
    unsafe fn create() -> &'static mut Self {
        Stk::new()
    }
}

/// Future returned by [`Stk::run`]
///
/// Should be immediatly polled when created and driven until finished.
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct StkFuture<'a, F, R> {
    inner: InnerStkFuture<'a, F, R, Stk>,
}

impl<'a, F, R> StkFuture<'a, F, R> {
    unsafe_pinned!(inner: InnerStkFuture<'a, F, R, Stk>);
}

impl<'a, F, Fut, R> Future for StkFuture<'a, F, R>
where
    F: FnOnce(&'a mut Stk) -> Fut,
    Fut: Future<Output = R> + 'a,
{
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let inner = self.inner();
        inner.poll(cx)
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
            inner: InnerStkFuture {
                f: Some(f),
                completed: false,
                res: UnsafeCell::new(None),
                _marker: PhantomData,
            },
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
    unsafe_unpinned!(stack: Stack);
    unsafe_unpinned!(place: NonNull<UnsafeCell<Option<R>>>);
}

impl<'a, R> Future for ScopeStkFuture<'a, R> {
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        with_stack_context(|parent_stack| {
            enter_stack_context(&this.stack, || loop {
                match this.stack.drive_head(cx) {
                    Poll::Ready(_) => {
                        if this.stack.tasks().is_empty() {
                            let res = unsafe { (*this.place.as_ref().get()).take().unwrap() };
                            return Poll::Ready(res);
                        }
                    }
                    Poll::Pending => match this.stack.get_state() {
                        State::Base => return Poll::Pending,
                        State::NewTask => this.stack.set_state(State::Base),
                        State::Yield => {
                            this.stack.set_state(State::Base);
                            parent_stack.set_state(State::Yield);
                            return Poll::Pending;
                        }
                        State::Cancelled => return Poll::Pending,
                    },
                }
            })
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
        let future = unsafe { f(Stk::new()) };

        let stack = Stack::new();
        let place = Box::into_raw(Box::new(UnsafeCell::new(None)));
        let place = unsafe { NonNull::new_unchecked(place) };

        stack
            .tasks()
            .push(async move { unsafe { place.as_ref().get().write(Some(future.await)) } });

        ScopeStkFuture {
            place,
            stack,
            _marker: PhantomData,
        }
    }
}
