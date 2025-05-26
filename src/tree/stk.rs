use std::{future::Future, marker::PhantomData};

use crate::{
    ptr::Owned,
    stack::{
        future::{InnerStkFuture, YieldFuture},
        StackMarker,
    },
    TreeStack,
};

use crate::tree::future::StkFuture;

use super::future::{ScopeFuture, ScopeStkFuture};

/// A reference back to stack from inside the running future.
///
/// Used for spawning new futures onto the stack from a future running on the stack.
pub struct Stk(PhantomData<*mut TreeStack>);

impl StackMarker for Stk {
    unsafe fn create() -> &'static mut Self {
        Owned::<Stk>::dangling().as_mut()
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

    /// A less type-safe version of Stk::run which doesn't require passing arround a Stk object.
    /// Invalid use of this function can cause a panic or deadlocking an executor.
    ///
    /// # Panic
    /// This function will panic while not within a TreeStack
    /// The future returned by this function will panic if another stack futures is created which
    /// is not contained within the future returned by this function while the current future is
    /// still running
    pub fn enter_run<'a, F, Fut, R>(f: F) -> StkFuture<'a, F, R>
    where
        F: FnOnce(&'a mut Stk) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        // Check if this is being run in the right context
        TreeStack::with_context(|_| ());
        StkFuture(InnerStkFuture::new(f))
    }

    /// Yield the execution of the recursive futures back to the reblessive runtime.
    ///
    /// When stepping through a function instead of finishing it awaiting the future returned by
    /// this function will cause the the current step to complete.
    pub fn yield_now(&mut self) -> YieldFuture {
        YieldFuture::new()
    }

    /// Create a scope in which multiple reblessive futures can be polled at the same time.
    pub fn scope<'a, F, Fut, R>(&'a mut self, f: F) -> ScopeFuture<'a, F, R>
    where
        F: FnOnce(&'a ScopeStk) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        ScopeFuture::new(f)
    }

    /// A less type-safe version of Stk::scope which doesn't require passing arround a Stk object.
    /// Invalid use of this function can cause a panic or deadlocking an executor.
    ///
    /// # Panic
    /// This function will panic while not within a TreeStack
    pub fn enter_scope<'a, F, Fut, R>(f: F) -> ScopeFuture<'a, F, R>
    where
        F: FnOnce(&'a ScopeStk) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        // Check if this is being run in the right context
        TreeStack::with_context(|_| ());
        ScopeFuture::new(f)
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
        Owned::dangling().as_mut()
    }
}

impl ScopeStk {
    /// Run a new future in the runtime.
    pub fn run<'a, F, Fut, R>(&'a self, f: F) -> ScopeStkFuture<'a, R>
    where
        F: FnOnce(&'a mut Stk) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        let future = unsafe { f(Stk::create()) };

        ScopeStkFuture::new(future)
    }
}
