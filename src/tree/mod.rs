//! A runtime which allows parallel running of branching futures.

use crate::{defer::Defer, ptr::Owned, stack::StackMarker, Stack};
use std::{
    cell::Cell,
    future::Future,
    task::{Context, Poll},
};

mod future;
mod runner;
mod schedular;
mod stk;

#[cfg(test)]
mod test;

pub use future::{ScopeFuture, StkFuture};
use runner::Runner;
pub use runner::{FinishFuture, StepFuture};
use schedular::Schedular;
pub use stk::Stk;

thread_local! {
    static TREE_PTR: Cell<Option<Owned<Schedular>>> = const { Cell::new(None) };
}

/// A runtime similar to [`Stack`] but allows running some futures in parallel
pub struct TreeStack {
    root: Stack,
    schedular: Schedular,
}

unsafe impl Send for TreeStack {}
unsafe impl Sync for TreeStack {}

impl TreeStack {
    pub fn new() -> Self {
        TreeStack {
            root: Stack::new(),
            schedular: Schedular::new(),
        }
    }

    pub fn enter<'a, F, Fut, R>(&'a mut self, f: F) -> Runner<R>
    where
        F: FnOnce(&'a mut Stk) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        let fut = unsafe { f(Stk::create()) };

        unsafe { self.root.enter_future(fut) };

        Runner::new(self)
    }

    pub(crate) fn enter_context<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let old = TREE_PTR.replace(Some(Owned::from(&self.schedular)));
        let _defer = Defer::new(old, |old| TREE_PTR.set(*old));
        f()
    }

    pub(crate) fn with_context<F, R>(f: F) -> R
    where
        F: FnOnce(&Schedular) -> R,
    {
        unsafe {
            f(TREE_PTR
                .get()
                .expect("Used TreeStack functions outside of TreeStack context")
                .as_ref())
        }
    }

    pub(crate) unsafe fn drive_top_task(&self, context: &mut Context) -> Poll<bool> {
        self.enter_context(|| {
            if !self.schedular.is_empty() {
                let pending = self
                    .root
                    .enter_context(|| self.schedular.poll(context).is_pending());
                if pending {
                    return Poll::Pending;
                }
            }
            self.root.drive_top_task(context)
        })
    }
}

impl Default for TreeStack {
    fn default() -> Self {
        Self::new()
    }
}
