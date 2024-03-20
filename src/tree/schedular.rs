use crate::task::BoxedTask;
use futures_util::FutureExt;
use std::{
    cell::RefCell,
    future::Future,
    task::{Context, Poll},
};

pub struct Schedular {
    inner: RefCell<Vec<Option<BoxedTask>>>,
}

impl Schedular {
    pub fn new() -> Self {
        Schedular {
            inner: RefCell::new(Vec::new()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.inner.borrow().is_empty()
    }

    /// # Safety
    /// This function erases any lifetime associated with the future.
    /// Caller must ensure that either the future completes or is dropped before the lifetime
    pub unsafe fn push<F>(&self, f: F)
    where
        F: Future<Output = ()>,
    {
        let f = unsafe { BoxedTask::new(f) };
        self.inner.borrow_mut().push(Some(f));
    }

    pub fn clear(&self) {
        self.inner.borrow_mut().clear()
    }

    pub fn poll(&self, cx: &mut Context) -> Poll<()> {
        let mut cur = 0;
        loop {
            let mut future = {
                let mut borrow = self.inner.borrow_mut();
                let Some(x) = borrow.get_mut(cur) else {
                    break;
                };
                x.take().unwrap()
            };
            match future.poll_unpin(cx) {
                Poll::Pending => {
                    self.inner.borrow_mut()[cur] = Some(future);
                }
                _ => {}
            }
            cur += 1;
        }

        let mut borrow = self.inner.borrow_mut();
        borrow.retain(|x| x.is_some());
        if borrow.is_empty() {
            Poll::Ready(())
        } else {
            Poll::Pending
        }
    }
}
