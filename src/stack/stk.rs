use crate::stack::{with_stack_context, Stack, State};
use std::{
    cell::UnsafeCell,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

pub trait StackMarker: 'static {
    unsafe fn create() -> &'static mut Self;
}

impl StackMarker for Stk {
    unsafe fn create() -> &'static mut Self {
        Stk::new()
    }
}

pub(crate) struct InnerStkFuture<'a, F, R, M> {
    // The function to execute to get the future
    pub(crate) f: Option<F>,
    pub(crate) running: bool,
    // The place where the future will store the result.
    pub(crate) res: UnsafeCell<Option<R>>,
    pub(crate) _marker: PhantomData<&'a mut M>,
}

impl<'a, F, Fut, R, M> Future for InnerStkFuture<'a, F, R, M>
where
    F: FnOnce(&'a mut M) -> Fut,
    Fut: Future<Output = R> + 'a,
    M: StackMarker,
{
    type Output = R;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // SAFETY: Pinning isn't structural for any of the fields.
        let this = unsafe { self.get_unchecked_mut() };
        unsafe {
            if let Some(x) = this.f.take() {
                // Closure is still present, this is the first call try to spawn a new future on
                // the stack.
                with_stack_context(|stack| {
                    // The state of the stack must be base, if not, a future has been spawned and
                    // spawning another could result in unsound behaviour.
                    assert_eq!(stack.get_state(), State::Base, "Tried to push a task while another stack future is running. Stack futures need be completed before another task can be ran.");

                    // Create a pointer to the result place.
                    // As this future is now pinned it cannot be moved until it is dropped so
                    // handing this pointer to the stack is sound as long as we make sure the task
                    // handling the pointer is dropped when this future is dropped.
                    let place = NonNull::from(&this.res);
                    let fut = (x)(M::create());

                    // If the context is not the one created at the root we have entered this
                    // future from a different schedular, this means that this future might not
                    // wake up when it needs to unless we wake the waker. However we don't always
                    // wan't to clone and run the waker cause that has significant performance
                    // impacts.
                    if stack.is_root_context(cx) {
                        stack.tasks.push(async move {
                            place.as_ref().get().write(Some(fut.await));
                        });
                    } else {
                        let waker = cx.waker().clone();

                        stack.tasks.push(async move {
                            place.as_ref().get().write(Some(fut.await));
                            waker.wake()
                        });
                    }

                    // Set the state to new task, signifying that no new future can be pushed and
                    // that we should yield back to stack executor.
                    stack.set_state(State::NewTask);
                    // We are now running the future so if this future is dropped before completion
                    // wwe also need to drop the task.
                    this.running = true;
                });
                return Poll::Pending;
            }

            // Executors could possibly poll this future again before it is actually completed
            // This should only happend while still in NewTask state, as after new task state is
            // cleared this future should not be ran again until its spawned task completed.
            if let Some(x) = (*this.res.get()).take() {
                // Set the this pointer to null to signal the drop impl that we don't need to pop
                // the task.
                this.running = false;
                return Poll::Ready(x);
            } else {
                #[cfg(debug_assertions)]
                if this.running {
                    with_stack_context(|stack| assert_eq!(stack.get_state(), State::NewTask))
                }
            }
        }
        Poll::Pending
    }
}

impl<'a, F, R, M> Drop for InnerStkFuture<'a, F, R, M> {
    fn drop(&mut self) {
        if self.running && self.res.get_mut().is_none() {
            // the future is still in a running state and its result was not yet set.
            // This means that the task is still present on the stack and therefore must be popped.
            with_stack_context(|stack| {
                if stack.get_state() != State::Cancelled {
                    unsafe { stack.tasks().pop() };
                }
            })
        }
    }
}

/// Future returned by [`Stk::run`]
///
/// Should be immediatly polled when created and driven until finished.
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct StkFuture<'a, F, R> {
    inner: InnerStkFuture<'a, F, R, Stk>,
}

impl<'a, F, Fut, R> Future for StkFuture<'a, F, R>
where
    F: FnOnce(&'a mut Stk) -> Fut,
    Fut: Future<Output = R> + 'a,
{
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Safety: Pinning is structural for inner.
        let inner = unsafe { self.map_unchecked_mut(|x| &mut x.inner) };
        inner.poll(cx)
    }
}

/// Future returned by [`Stk::yield_now`]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct YieldFuture {
    pub(crate) done: bool,
}

impl Future for YieldFuture {
    type Output = ();

    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.get_mut();
        with_stack_context(|stack| {
            if !this.done {
                this.done = true;
                // Set the state to yield. We must set this state because a Poll::Pending without
                // an associated state change is interpreted as an external future returning
                // pending so we need to yield back
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
            inner: InnerStkFuture {
                running: false,
                f: Some(f),
                res: UnsafeCell::new(None),
                _marker: PhantomData,
            },
        }
    }

    /// A less save version of Stk::run which doesn't require passing arround a Stk object.
    /// Invalid use of this function will cause a panic, deadlock or otherwise generally sound but
    /// strange behaviour.
    ///
    ///
    /// # Panic
    /// This function will panic while not within a Stack
    /// The future returned by this function will panic if another stack futures is created which
    /// is not contained within the future returned by this function while the current future is
    /// still running
    pub fn enter_run<'a, F, Fut, R>(f: F) -> StkFuture<'a, F, R>
    where
        F: FnOnce(&'a mut Stk) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        StkFuture {
            inner: InnerStkFuture {
                f: Some(f),
                running: false,
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
}
