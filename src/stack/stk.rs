use super::future::{InnerStkFuture, StkFuture, YieldFuture};
use crate::{ptr::Owned, Stack};
use std::{future::Future, marker::PhantomData};

/// A reference back to stack from inside the running future.
///
/// Used for spawning new futures onto the stack from a future running on the stack.
pub struct Stk(
    // Marker to make sure Stk doesn't implement send or sync.
    PhantomData<*mut Stack>,
);

pub trait StackMarker: 'static {
    unsafe fn create() -> &'static mut Self;
}

impl StackMarker for Stk {
    unsafe fn create() -> &'static mut Self {
        // Safety: Stk is an unsized typed so any pointer that is not null is a valid pointer to the type.
        // Therefore we can create a static reference to the type from a dangling pointer.
        unsafe { Owned::dangling().as_mut() }
    }
}

impl Stk {
    /// Run a new future in the runtime.
    pub fn run<'a, F, Fut, R>(&'a mut self, f: F) -> StkFuture<'a, F, R>
    where
        F: FnOnce(&'a mut Stk) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        StkFuture(InnerStkFuture::new(f))
    }

    /// A less type save version of Stk::run which doesn't require passing arround a Stk object.
    /// Invalid use of this function will cause a panic, deadlock or otherwise generally sound but
    /// strange behaviour.
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
        StkFuture(InnerStkFuture::new(f))
    }

    /// Yield the execution of the recursive futures back to the reblessive runtime.
    ///
    /// When stepping through a function instead of finishing it awaiting the future returned by
    /// this function will cause the the current step to complete.
    pub fn yield_now(&mut self) -> YieldFuture {
        YieldFuture::new()
    }
}
