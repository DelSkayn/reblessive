use std::{
    cell::Cell,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use crate::{ptr::Owned, Stack};

use super::{
    stk::{StackMarker, Stk},
    StackState,
};

pub enum StkFutureState<F, R> {
    Initial(F),
    Running(Cell<Option<R>>),
    Done,
}

pub(crate) struct InnerStkFuture<'a, F, R, M> {
    pub(crate) state: StkFutureState<F, R>,
    pub(crate) _marker: PhantomData<&'a mut M>,
}

impl<'a, F, R, M> InnerStkFuture<'a, F, R, M> {
    pub fn new(f: F) -> Self {
        InnerStkFuture {
            state: StkFutureState::Initial(f),
            _marker: PhantomData,
        }
    }
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
        let this = unsafe { self.get_unchecked_mut() };
        Stack::with_context(|stack| match this.state {
            StkFutureState::Initial(_) => {
                // match in two steps so we don't move out of state until we are sure we can move
                // out.
                let StkFutureState::Initial(closure) =
                    std::mem::replace(&mut this.state, StkFutureState::Running(Cell::new(None)))
                else {
                    unreachable!();
                };

                let StkFutureState::Running(ref mut state) = this.state else {
                    unreachable!();
                };

                let state_ptr = Owned::from(state);

                // Safety: M::create is save here because we are running in the reblessive runtime.
                let future = unsafe { closure(M::create()) };

                // todo check for sub schedulars
                if stack.is_rebless_context(cx) {
                    unsafe {
                        stack.push_task(async move {
                            state_ptr.as_ref().set(Some(future.await));
                        })
                    }
                } else {
                    // if the context has changed there is probably a sub schedular inbetween this
                    // future and the reblessive runtime. We should notify this schedular when this
                    // future is ready.
                    let waker = cx.waker().clone();
                    unsafe {
                        stack.push_task(async move {
                            state_ptr.as_ref().set(Some(future.await));
                            waker.wake()
                        })
                    }
                }
                return Poll::Pending;
            }
            StkFutureState::Running(ref closure) => {
                let Some(x) = closure.take() else {
                    return Poll::Pending;
                };

                this.state = StkFutureState::Done;

                return Poll::Ready(x);
            }
            StkFutureState::Done => return Poll::Pending,
        })
    }
}

impl<'a, F, R, M> Drop for InnerStkFuture<'a, F, R, M> {
    fn drop(&mut self) {
        match self.state {
            // Never polled
            StkFutureState::Initial(_) => {}
            // Finised execution
            StkFutureState::Done => {}
            // Dropped after polled, might only be partially finished.
            StkFutureState::Running(ref r) => {
                if r.take().is_none() {
                    // r.take is none so it's parent future hasn't finished yet.
                    Stack::with_context(|stack| {
                        // make sure the task didn't already get dropped.
                        if stack.state.get() != StackState::Cancelled {
                            unsafe { stack.pop_cancel_task() };
                        }
                    })
                }
            }
        }
    }
}

/// Future returned by [`Stk::run`]
///
/// Should be immediatly polled when created and driven until finished.
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct StkFuture<'a, F, R>(pub(crate) InnerStkFuture<'a, F, R, Stk>);
impl<'a, F, Fut, R> Future for StkFuture<'a, F, R>
where
    F: FnOnce(&'a mut Stk) -> Fut,
    Fut: Future<Output = R> + 'a,
{
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe { self.map_unchecked_mut(|t| &mut t.0).poll(cx) }
    }
}

pub struct YieldFuture<'a>(bool, PhantomData<&'a mut Stk>);

impl YieldFuture<'_> {
    pub(crate) fn new<'a>() -> YieldFuture<'a> {
        YieldFuture(false, PhantomData)
    }
}

impl Future for YieldFuture<'_> {
    type Output = ();

    fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        Stack::with_context(|stack| {
            if !self.0 && matches!(stack.state.get(), StackState::Yield) {
                self.0 = true;
                if !stack.is_rebless_context(cx) {
                    cx.waker().wake_by_ref()
                }
                let s = stack.state.replace(StackState::Yield);
                assert_eq!(
                    s,
                    StackState::Base,
                    "Stack was in inconsistant state for yielding."
                );
                return Poll::Pending;
            }
            Poll::Ready(())
        })
    }
}
