use super::{schedular::CancelToken, stk::ScopeStk, Stk};
use crate::{
    map_ptr,
    ptr::{Mut, Owned},
    stack::{future::InnerStkFuture, StackState},
    Stack, TreeStack,
};
use std::{
    cell::Cell,
    future::Future,
    marker::{PhantomData, PhantomPinned},
    pin::Pin,
    task::{Context, Poll},
};

/// Future returned by [`Stk::run`]
///
/// Should be finished completely after the first polling before any other futures returned by [`Stk`] are polled.
/// Failing to do so will cause a panic.
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct StkFuture<'a, F, R>(pub(crate) InnerStkFuture<'a, F, R, Stk>);

impl<'a, F, Fut, R> Future for StkFuture<'a, F, R>
where
    F: FnOnce(&'a mut Stk) -> Fut,
    Fut: Future<Output = R> + 'a,
{
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        // Safety: Pinning is structural for inner
        unsafe { self.map_unchecked_mut(|x| &mut x.0) }.poll(cx)
    }
}

pub enum ScopeFutureState<F, R> {
    Initial(F),
    Running(Cell<Option<R>>, Cell<Option<CancelToken>>),
    Finished,
}

/// Future returned by [`Stk::scope`]
///
/// Should be finished completely after the first polling before any other futures returned by [`Stk`] are polled.
/// Failing to do so will cause a panic.
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct ScopeFuture<'a, F, R> {
    state: ScopeFutureState<F, R>,
    _marker: PhantomData<&'a Stk>,
    _pin_marker: PhantomPinned,
}

impl<'a, F, R> ScopeFuture<'a, F, R> {
    pub(crate) fn new(f: F) -> Self {
        ScopeFuture {
            state: ScopeFutureState::Initial(f),
            _marker: PhantomData,
            _pin_marker: PhantomPinned,
        }
    }
}

impl<'a, F, Fut, R> Future for ScopeFuture<'a, F, R>
where
    F: FnOnce(&'a ScopeStk) -> Fut,
    Fut: Future<Output = R> + 'a,
    R: Default + 'a,
{
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        unsafe {
            let mut this = Mut::from(self.get_unchecked_mut());
            match this.as_ref().state {
                ScopeFutureState::Initial(_) => {
                    let ScopeFutureState::Initial(x) = this
                        .reborrow()
                        .map_ptr(map_ptr!(Self, state))
                        .replace(ScopeFutureState::Running(Cell::new(None), Cell::new(None)))
                    else {
                        unreachable!()
                    };
                    let ScopeFutureState::Running(ref place, ref cancel) =
                        this.map_ptr(map_ptr!(Self, state)).as_ref()
                    else {
                        unreachable!();
                    };

                    let place = Owned::from(place);
                    let fut = x(ScopeStk::new());

                    let cancel_token = Stack::with_context(|x| {
                        x.state.set(StackState::NewTask);
                        if x.is_rebless_context(cx) {
                            TreeStack::with_context(|sched| {
                                sched.push_cancellable(async move {
                                    place.as_ref().set(Some(fut.await));
                                })
                            })
                        } else {
                            TreeStack::with_context(|sched| {
                                let waker = cx.waker().clone();
                                let r = sched.push_cancellable(async move {
                                    place.as_ref().set(Some(fut.await));
                                    waker.wake()
                                });
                                r
                            })
                        }
                    });
                    cancel.set(Some(cancel_token));

                    Poll::Pending
                }
                ScopeFutureState::Running(ref place, _) => {
                    if let Some(x) = place.take() {
                        let _ = this
                            .map_ptr(map_ptr!(Self, state))
                            .replace(ScopeFutureState::Finished);
                        return Poll::Ready(x);
                    }
                    Poll::Pending
                }
                ScopeFutureState::Finished => return Poll::Pending,
            }
        }
    }
}

impl<F, R> Drop for ScopeFuture<'_, F, R> {
    fn drop(&mut self) {
        match self.state {
            ScopeFutureState::Running(_, ref mut c) => {
                // drop the cancellation so that the future won't run anymore.
                // Manually dropped first so that the future is always dropped before the place is.
                std::mem::drop(c.take());
            }
            _ => {}
        }
    }
}

#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct ScopeStkFuture<'a, R> {
    stack: Stack,
    _marker_borrow: PhantomData<&'a ScopeStk>,
    _marker_res: PhantomData<R>,
}

impl<R> ScopeStkFuture<'_, R> {
    pub(crate) fn new<F>(f: F) -> Self
    where
        F: Future<Output = R>,
    {
        let stack = Stack::new();
        unsafe { stack.enter_future(f) };
        ScopeStkFuture {
            stack,
            _marker_borrow: PhantomData,
            _marker_res: PhantomData,
        }
    }
}

impl<'a, R> Future for ScopeStkFuture<'a, R> {
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = unsafe { self.get_unchecked_mut() };
        Stack::with_context(|parent| {
            // Make sure we immediatlu yield if we need to yield back to the parent stack.
            if parent.state.get() == StackState::Yield {
                cx.waker().wake_by_ref();
                return Poll::Pending;
            }

            Stack::enter_context(&this.stack, || loop {
                if let Some(x) = unsafe { this.stack.try_get_result() } {
                    return Poll::Ready(x);
                }

                match unsafe { this.stack.drive_top_task(cx) } {
                    Poll::Pending => return Poll::Pending,
                    Poll::Ready(yielding) => {
                        if yielding {
                            parent.state.set(StackState::Yield);
                            cx.waker().wake_by_ref();
                            return Poll::Pending;
                        }
                    }
                }
            })
        })
    }
}
