//! The stack runtime
//!
//! A runtime for turning recursive functions into a number of futures which are run from a single
//! flattened loop, preventing stack overflows.
//!
//! This runtime also has support for external async function but it explicitly doesn't support
//! intra-task concurrency, i.e. calling select or join on multiple futures at the same time. These
//! types of patterns break the stack allocation pattern which this executor uses to be able to
//! allocate and run futures efficiently.

use crate::{stub_ctx::WakerCtx, task::Tasks};
use pin_project_lite::pin_project;
use std::{
    cell::{Cell, UnsafeCell},
    future::Future,
    marker::PhantomData,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

mod stk;
use stk::TaskFuture;
pub use stk::{Stk, StkFuture, YieldFuture};

#[cfg(test)]
mod test;

pin_project! {
    /// Future returned by [`Runner::finish_async`]
    #[must_use]
    pub struct FinishFuture<'a,R>{
        runner: Runner<'a,R>
    }
}

impl<'a, R> Future for FinishFuture<'a, R> {
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        unsafe {
            let tasks = &this.runner.ptr.tasks;

            let waker_ctx = WakerCtx {
                stack: this.runner.ptr,
                waker: Some(cx.waker()),
            };
            let waker = waker_ctx.to_waker();
            let mut cx = Context::from_waker(&waker);

            loop {
                let Some(mut task) = tasks.last() else {
                    panic!("Tasks empty")
                };

                loop {
                    match task.drive(&mut cx) {
                        Poll::Pending => match this.runner.stack_state() {
                            State::Base => return Poll::Pending,
                            State::NewTask => {
                                // New task was pushed so we need to start driving that task.
                                this.runner.set_stack_state(State::Base);
                                break;
                            }
                            State::Yield => {
                                // Yield was requested but no new task was pushed so continue.
                                this.runner.set_stack_state(State::Base);
                            }
                        },
                        Poll::Ready(_) => {
                            tasks.pop();
                            if tasks.is_empty() {
                                let value = (*this.runner.place.as_ref().get()).take().unwrap();
                                return Poll::Ready(value);
                            }
                            break;
                        }
                    }
                }
            }
        }
    }
}

pin_project! {
    /// Future returned by [`Runner::step_async`]
    #[must_use]
    pub struct StepFuture<'a,'b,R>{
        runner: &'a mut Runner<'b,R>
    }
}

impl<'a, 'b, R> Future for StepFuture<'a, 'b, R> {
    type Output = Option<R>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        unsafe {
            let waker_ctx = WakerCtx {
                stack: this.runner.ptr,
                waker: Some(cx.waker()),
            };
            let waker = waker_ctx.to_waker();
            let mut cx = Context::from_waker(&waker);

            match this.runner.ptr.drive_head(&mut cx) {
                Poll::Pending => {
                    match this.runner.ptr.state.get() {
                        State::Base => {
                            // A poll::pending was returned but no new task was created.
                            // Thus we are waiting on an external future, and need to return
                            // Poll::pending.
                            return Poll::Pending;
                        }
                        State::NewTask => {
                            // Poll::Pending was returned and a new future was created, therefore
                            // we need to continue evaluating tasks so return Poll::Ready
                            Poll::Ready(None)
                        }
                        State::Yield => {
                            // Poll::Pending was returned and with a request to interrupt execution
                            // so return ready
                            Poll::Ready(None)
                        }
                    }
                }
                Poll::Ready(_) => {
                    if this.runner.ptr.tasks().is_empty() {
                        return Poll::Ready(Some(
                            (*this.runner.place.as_ref().get()).take().unwrap(),
                        ));
                    }
                    Poll::Ready(None)
                }
            }
        }
    }
}

/// Struct returned by [`Stack::enter`] determines how futures should be ran.
pub struct Runner<'a, R> {
    place: NonNull<UnsafeCell<Option<R>>>,
    ptr: &'a Stack,
    _stack_marker: PhantomData<&'a mut Stack>,
    _res_marker: PhantomData<R>,
}

unsafe impl<'a, R> Send for Runner<'a, R> {}
unsafe impl<'a, R> Sync for Runner<'a, R> {}

impl<'a, R> Runner<'a, R> {
    fn stack_state(&self) -> State {
        self.ptr.state.get()
    }

    fn set_stack_state(&self, state: State) {
        self.ptr.state.set(state)
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
        unsafe { self.finish_inner() }
    }

    unsafe fn finish_inner(&mut self) -> R {
        let context = WakerCtx {
            stack: self.ptr,
            waker: None,
        };
        let waker = context.to_waker();
        let mut context = Context::from_waker(&waker);

        while let Some(mut task) = self.ptr.tasks.last() {
            loop {
                match task.drive(&mut context) {
                    Poll::Pending => match self.stack_state() {
                        State::Yield => {
                            self.set_stack_state(State::Base);
                        }
                        State::Base => {}
                        State::NewTask => {
                            self.set_stack_state(State::Base);
                            break;
                        }
                    },
                    Poll::Ready(_) => {
                        self.ptr.tasks.pop();
                        break;
                    }
                }
            }
        }
        (*self.place.as_ref().get()).take().unwrap()
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
        unsafe {
            let Some(mut task) = self.ptr.tasks.last() else {
                panic!("tasks already empty");
            };

            let context = WakerCtx {
                stack: self.ptr,
                waker: None,
            };
            let waker = context.to_waker();
            let mut context = Context::from_waker(&waker);

            match task.drive(&mut context) {
                Poll::Pending => match self.stack_state() {
                    State::Base => {}
                    State::Yield | State::NewTask => {
                        self.set_stack_state(State::Base);
                    }
                },
                Poll::Ready(_) => {
                    self.ptr.tasks.pop();
                    if self.ptr.tasks.len() == 0 {
                        return Some((*self.place.as_ref().get()).take().unwrap());
                    }
                }
            }
        }
        None
    }

    /// Run the spawned future for a single step, returning none if a future either completed or
    /// spawned a new future onto the stack. Will return some if the root future is finished.
    ///
    /// This function supports sleeping or taking ownership of the waker allowing it to be used
    /// with external async runtimes.
    pub fn step_async<'b>(&'b mut self) -> StepFuture<'b, 'a, R> {
        StepFuture { runner: self }
    }

    /// Returns the number of futures currently spawned on the stack.
    pub fn depth(&self) -> usize {
        self.ptr.tasks.len()
    }

    /// Drive the stack until it completes.
    ///
    /// This function supports cloning and awakening allowing it to be used with external async
    /// runtimes
    pub fn finish_async(self) -> FinishFuture<'a, R> {
        FinishFuture { runner: self }
    }
}

impl<'a, R> Drop for Runner<'a, R> {
    fn drop(&mut self) {
        self.ptr.clear();
        unsafe { std::mem::drop(Box::from_raw(self.place.as_ptr())) };
    }
}

#[derive(Clone, Copy, Debug, Eq,PartialEq)]
pub(crate) enum State {
    /// normal execution of the stack.
    Base,
    /// A new task was pushed to the Stack
    /// the current running future should yield back to the stack to continue executing the current
    /// future.
    NewTask,
    /// Yielding was requested by a future.
    Yield,
}

/// A small minimal runtime for executing futures flattened onto the heap preventing stack
/// overflows on deeply nested futures. Only capable of running a single future at the same time
/// and has no support for waking tasks by itself.
pub struct Stack {
    state: Cell<State>,
    tasks: Tasks,
}

unsafe impl Send for Stack {}
unsafe impl Sync for Stack {}

impl Stack {
    /// Create a new empty stack to run reblessive futures in.
    ///
    /// This function does not allocate.
    pub fn new() -> Self {
        Stack {
            state: Cell::new(State::Base),
            tasks: Tasks::new(),
        }
    }

    /// Create a new empty stack to run reblessive futures in with atleast cap bytes reserved for
    /// future allocation.
    pub fn with_capacity(cap: usize) -> Self {
        Stack {
            state: Cell::new(State::Base),
            tasks: Tasks::with_capacity(cap),
        }
    }

    /// Run a future in the stack.
    pub fn enter<'a, F, Fut, R>(&'a mut self, f: F) -> Runner<'a, R>
    where
        F: FnOnce(&'a mut Stk) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        assert!(
            self.tasks.is_empty(),
            "Stack left in inconsistent state, was a previous runner leaked?"
        );
        unsafe {
            let ctx = Stk::new();

            let place = Box::new(UnsafeCell::new(None));
            let place_ptr = NonNull::new_unchecked(Box::into_raw(place));

            self.tasks.push(TaskFuture {
                place: place_ptr,
                inner: (f)(ctx),
            });

            Runner {
                place: place_ptr,
                ptr: self,
                _stack_marker: PhantomData,
                _res_marker: PhantomData,
            }
        }
    }

    pub(crate) fn drive_head(&self, cx: &mut Context) -> Poll<()> {
        let Some(mut task) = self.tasks.last() else {
            panic!("Missing tasks");
        };

        match unsafe { task.drive(cx) } {
            Poll::Pending => Poll::Pending,
            Poll::Ready(_) => {
                unsafe { self.tasks.pop() };
                Poll::Ready(())
            }
        }
    }

    pub(crate) fn tasks(&self) -> &Tasks {
        &self.tasks
    }

    pub(crate) fn get_state(&self) -> State {
        self.state.get()
    }

    pub(crate) fn set_state(&self, state: State) {
        self.state.set(state)
    }

    pub(crate) fn clear(&self){
        self.tasks.clear();
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self::new()
    }
}