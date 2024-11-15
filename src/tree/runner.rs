use std::{
    future::Future,
    marker::PhantomData,
    pin::Pin,
    task::{Context, Poll},
};

use crate::TreeStack;

#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct FinishFuture<'a, R> {
    stack: &'a TreeStack,
    _marker: PhantomData<R>,
}

impl<R> Future for FinishFuture<'_, R> {
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let addr = self.stack.root.set_rebless_context(cx);
        loop {
            if let Some(x) = unsafe { self.stack.root.try_get_result() } {
                self.stack.root.set_rebless_context_addr(addr);
                return Poll::Ready(x);
            }

            let Poll::Ready(_) = (unsafe { self.stack.drive_top_task(cx) }) else {
                self.stack.root.set_rebless_context_addr(addr);
                return Poll::Pending;
            };
        }
    }
}

#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct StepFuture<'a, R> {
    stack: &'a TreeStack,
    _marker: PhantomData<R>,
}

impl<R> Future for StepFuture<'_, R> {
    type Output = Option<R>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let addr = self.stack.root.set_rebless_context(cx);
        if let Some(x) = unsafe { self.stack.root.try_get_result() } {
            self.stack.root.set_rebless_context_addr(addr);
            return Poll::Ready(Some(x));
        }

        let Poll::Ready(_) = (unsafe { self.stack.drive_top_task(cx) }) else {
            self.stack.root.set_rebless_context_addr(addr);
            return Poll::Pending;
        };
        Poll::Ready(None)
    }
}

pub struct Runner<'a, R> {
    stack: &'a TreeStack,
    _marker: PhantomData<R>,
}

unsafe impl<R> Send for Runner<'_, R> {}
unsafe impl<R> Sync for Runner<'_, R> {}

impl<'a, R> Runner<'a, R> {
    pub(crate) fn new(runner: &'a TreeStack) -> Self {
        Runner {
            stack: runner,
            _marker: PhantomData,
        }
    }

    pub fn finish(self) -> FinishFuture<'a, R> {
        let res = FinishFuture {
            stack: self.stack,
            _marker: PhantomData,
        };
        std::mem::forget(self);
        res
    }

    pub fn step(&mut self) -> StepFuture<R> {
        StepFuture {
            stack: self.stack,
            _marker: PhantomData,
        }
    }
}

impl<R> Drop for Runner<'_, R> {
    fn drop(&mut self) {
        self.stack.schedular.clear();
        unsafe {
            self.stack.root.clear::<R>();
        }
    }
}
