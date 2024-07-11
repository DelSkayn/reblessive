use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use crate::{stub_waker, Stack};

use super::StackState;

/// Struct returned by [`Stack::enter`] determines how futures should be ran.
pub struct Runner<'a, R> {
    pub(crate) stack: &'a Stack,
    pub(crate) _marker: PhantomData<R>,
}

impl<'a, R> Runner<'a, R> {
    /// Run the spawned future for a single step, returning none if a future either completed or
    /// spawned a new future onto the stack. Will return some if the root future is finished.
    ///
    /// This function supports sleeping or taking ownership of the waker allowing it to be used
    /// with external async runtimes.
    pub fn step_async(&mut self) -> StepFuture<'a, R> {
        StepFuture {
            stack: self.stack,
            _marker: PhantomData,
        }
    }

    /// Drive the stack until it completes.
    ///
    /// This function supports cloning and awakening allowing it to be used with external async
    /// runtimes
    pub fn finish_async(&mut self) -> FinishFuture<'a, R> {
        FinishFuture {
            stack: self.stack,
            _marker: PhantomData,
        }
    }

    /// Run the spawned future for a single step, returning none if a future either completed or
    /// spawned a new future onto the stack. Will return some if the root future is finished.
    ///
    /// # Panics
    ///
    /// This function will panic if the waker inside the future running on the stack either tries
    /// to clone the waker or tries to call wake. This function is not meant to used with any other
    /// future except those generated with the various function provided by the stack. For the
    /// async version see [`Runner::step_async`]
    pub fn step(&mut self) -> Option<R> {
        if let Some(x) = unsafe { self.stack.try_get_result::<R>() } {
            return Some(x);
        }

        let waker = stub_waker::get();
        let mut context = Context::from_waker(&waker);
        let rebless_addr = self.stack.set_rebless_context(&mut context);

        match unsafe { self.stack.drive_top_task(&mut context) } {
            Poll::Pending => {
                panic!("a non-reblessive future was run while running the reblessive runtime as non-async")
            }
            _ => {}
        }

        self.stack.set_rebless_context_addr(rebless_addr);
        None
    }

    /// Drive the stack until it completes.
    ///
    /// # Panics
    ///
    /// This function will panic if the waker inside the future running on the stack either tries
    /// to clone the waker or tries to call wake. This function is not meant to used with any other
    /// future except those generated with the various function provided by the stack. For the
    /// async version see [`Runner::finish_async`]
    pub fn finish(mut self) -> R {
        loop {
            if let Some(x) = self.step() {
                return x;
            }
        }
    }

    /// Returns the number of futures currently spawned on the stack.
    pub fn depth(&self) -> usize {
        self.stack.pending_tasks()
    }
}

impl<R> Drop for Runner<'_, R> {
    fn drop(&mut self) {
        // The runner is dropped so we need to clear all the futures that might still be present on
        // the stack, if the runner was dropped before finishing the stack.
        self.stack.state.set(StackState::Cancelled);
        unsafe { self.stack.clear::<R>() }
        self.stack.state.set(StackState::Base);
    }
}

/// Future returned by [`Runner::step_async`]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct StepFuture<'a, R> {
    stack: &'a Stack,
    _marker: PhantomData<R>,
}

impl<R> Future for StepFuture<'_, R> {
    type Output = Option<R>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(x) = unsafe { self.stack.try_get_result() } {
            return Poll::Ready(Some(x));
        }

        let rebless_addr = self.stack.set_rebless_context(cx);
        let r = unsafe { self.stack.drive_top_task(cx) };
        self.stack.set_rebless_context_addr(rebless_addr);
        match r {
            Poll::Ready(_) => return Poll::Ready(None),
            Poll::Pending => return Poll::Pending,
        }
    }
}

/// Future returned by [`Runner::finish_async`]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct FinishFuture<'a, R> {
    stack: &'a Stack,
    _marker: PhantomData<R>,
}

impl<R> Future for FinishFuture<'_, R> {
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        loop {
            if let Some(x) = unsafe { self.stack.try_get_result() } {
                return Poll::Ready(x);
            }

            let rebless_addr = self.stack.set_rebless_context(cx);
            let r = unsafe { self.stack.drive_top_task(cx) };
            self.stack.set_rebless_context_addr(rebless_addr);
            match r {
                Poll::Ready(_) => {}
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}
