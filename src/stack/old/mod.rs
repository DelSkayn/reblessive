//! The stack runtime
//!
//! A runtime for turning recursive functions into a number of futures which are run from a single
//! flattened loop, preventing stack overflows.
//!
//! This runtime also has support for external async function but it explicitly doesn't support
//! intra-task concurrency, i.e. calling select or join on multiple futures at the same time. These
//! types of patterns break the stack allocation pattern which this executor uses to be able to
//! allocate and run futures efficiently.

use crate::{defer::Defer, ptr::Owned, stub_ctx};
use std::{
    borrow::BorrowMut,
    cell::{Cell, UnsafeCell},
    future::Future,
    marker::PhantomData,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

mod stk;
#[cfg(feature = "tree")]
pub(crate) use stk::{InnerStkFuture, StackMarker};
pub use stk::{Stk, StkFuture, YieldFuture};

mod task;
use task::StackTasks;

#[cfg(test)]
mod test;

/// Future returned by [`Runner::finish_async`]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct FinishFuture<'a, R> {
    runner: Runner<'a, R>,
}

impl<'a, R> Future for FinishFuture<'a, R> {
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        enter_stack_context(self.runner.ptr, || {
            unsafe {
                let tasks = &self.runner.ptr.tasks;
                let old_ctx = self.runner.ptr.set_context(Owned::from(&*cx).cast());
                let _defer_context = Defer::new(self.runner.ptr, |ptr| {
                    ptr.set_context(old_ctx);
                });

                loop {
                    let Some(mut task) = tasks.last() else {
                        panic!("Tasks empty")
                    };

                    loop {
                        let defer = Defer::new(tasks, |tasks| tasks.pop());

                        match task.drive(cx) {
                            // What is happing in a task depends on both what it returns and the
                            // state of the stack.
                            // If a task returns Poll::ready it has completed, needs to be popped,
                            // and the task before it must be executed again.
                            // For pending it depends on the current state.
                            // - NewTask: A future pushed a new task onto the stack and then
                            // yielded back. This means we need to run the new task.
                            // - Yield: A future requested that we yield back to the stack running.
                            // - Cancelled: The current stack is being dropped, this should be
                            // impossible while driving the future.
                            // - Base: All reblessive futures change the stack state before
                            // returning Poll::Pending. If no state change happend a non-reblessive
                            // future must have returned Poll::pending, we therefore need to yield
                            // back to a parent executor.
                            Poll::Pending => {
                                defer.take();
                                match self.runner.ptr.state.get() {
                                    State::Base => return Poll::Pending,
                                    State::NewTask => {
                                        // New task was pushed so we need to start driving that task.
                                        self.runner.ptr.state.set(State::Base);
                                        break;
                                    }
                                    State::Yield => {
                                        // Yield was requested but no new task was pushed so continue.
                                        self.runner.ptr.state.set(State::Base);
                                    }
                                    State::Cancelled => {
                                        unreachable!("Stack being dropped while actively driven")
                                    }
                                }
                            }
                            Poll::Ready(_) => {
                                std::mem::drop(defer);
                                if tasks.is_empty() {
                                    let value = (*self.runner.place.as_ref().get()).take().unwrap();
                                    return Poll::Ready(value);
                                }
                                break;
                            }
                        }
                    }
                }
            }
        })
    }
}

/// Future returned by [`Runner::step_async`]
#[must_use = "futures do nothing unless you `.await` or poll them"]
pub struct StepFuture<'a, 'b, R> {
    runner: &'a mut Runner<'b, R>,
}

impl<'a, 'b, R> Future for StepFuture<'a, 'b, R> {
    type Output = Option<R>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        enter_stack_context(self.runner.ptr, || {
            let old_ctx = self.runner.ptr.set_context(Owned::from(&*cx).cast());
            let _defer_context = Defer::new(self.runner.ptr, |ptr| {
                ptr.set_context(old_ctx);
            });
            unsafe {
                match self.runner.ptr.drive_head(cx) {
                    Poll::Pending => {
                        match self.runner.ptr.state.get() {
                            State::Base => {
                                // A poll::pending was returned but no new task was created.
                                // Thus we are waiting on an external future, and need to return
                                // Poll::pending.
                                return Poll::Pending;
                            }
                            State::Cancelled => {
                                unreachable!("Stack being dropped while actively driven")
                            }
                            State::NewTask => {
                                self.runner.ptr.state.set(State::Base);
                                // Poll::Pending was returned and a new future was created, therefore
                                // we need to continue evaluating tasks so return Poll::Ready
                                Poll::Ready(None)
                            }
                            State::Yield => {
                                self.runner.ptr.state.set(State::Base);
                                // Poll::Pending was returned and with a request to interrupt execution
                                // so return ready
                                Poll::Ready(None)
                            }
                        }
                    }
                    Poll::Ready(_) => {
                        if self.runner.ptr.pending_tasks_count() > 0 {
                            return Poll::Ready(Some(
                                (*self.runner.place.as_ref().get()).take().unwrap(),
                            ));
                        }
                        Poll::Ready(None)
                    }
                }
            }
        })
    }
}

/// Struct returned by [`Stack::enter`] determines how futures should be ran.
pub struct Runner<'a, R> {
    place: Owned<UnsafeCell<Option<R>>>,
    ptr: &'a Stack,
    _stack_marker: PhantomData<&'a mut Stack>,
    _res_marker: PhantomData<R>,
}

unsafe impl<'a, R> Send for Runner<'a, R> {}
unsafe impl<'a, R> Sync for Runner<'a, R> {}

impl<'a, R> Runner<'a, R> {
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
        enter_stack_context(self.ptr, || {
            let waker = stub_ctx::get();
            let mut context = Context::from_waker(&waker);
            let old_ctx = self.ptr.set_context(Owned::from(&context).cast());

            let _defer_context = Defer::new(self.ptr, |ptr| {
                ptr.set_context(old_ctx);
            });

            let task_borrow = Defer::new(self.ptr, |this| this.tasks.pop());

            while let Some(mut task) = task_borrow.tasks.last() {
                loop {
                    match task.drive(&mut context) {
                        Poll::Pending => match self.ptr.state.get() {
                            State::Yield => {
                                self.ptr.state.set(State::Base);
                            }
                            State::Base => {
                                panic!("Recieved a pending request from a non-reblessive future")
                            }
                            State::NewTask => {
                                self.ptr.state.set(State::Base);
                                break;
                            }
                            State::Cancelled => {
                                unreachable!("Stack being dropped while actively driven.")
                            }
                        },
                        Poll::Ready(_) => {
                            task_borrow.tasks.pop();
                            break;
                        }
                    }
                }
            }
            task_borrow.take();
            (*self.place.as_ref().get()).take().unwrap()
        })
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
        enter_stack_context(self.ptr, || {
            unsafe {
                let waker = stub_ctx::get();
                let mut context = Context::from_waker(&waker);
                let old_ctx = self.ptr.set_context(Owned::from(&context).cast());
                let _defer_context = Defer::new(self.ptr, |ptr| {
                    ptr.set_context(old_ctx);
                });

                match self.ptr.drive_head(&mut context) {
                    Poll::Pending => match self.ptr.state.get() {
                        State::Base => {
                            panic!("Recieved a pending request from a non-reblessive future")
                        }
                        State::Yield | State::NewTask => {
                            self.ptr.state.set(State::Base);
                        }
                        State::Cancelled => unreachable!("Stack dropped while being stepped"),
                    },
                    Poll::Ready(_) => {
                        if self.ptr.pending_tasks_count() == 0 {
                            return Some((*self.place.as_ref().get()).take().unwrap());
                        }
                    }
                }
            }
            None
        })
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
        self.ptr.pending_tasks_count()
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum State {
    /// normal execution of the stack.
    Base,
    /// A new task was pushed to the Stack
    /// the current running future should yield back to the stack to continue executing the current
    /// future.
    NewTask,
    /// Yielding was requested by a future.
    Yield,
    /// State used when the stack is being dropped and all the futures should be cancelledd.
    Cancelled,
}

thread_local! {
    static STACK_PTR: Cell<Option<NonNull<Stack>>> = const { Cell::new(None) };
}

pub(crate) fn enter_stack_context<F, R>(context: &Stack, f: F) -> R
where
    F: FnOnce() -> R,
{
    let ptr = STACK_PTR.with(|x| x.replace(Some(NonNull::from(context))));
    struct Dropper(Option<NonNull<Stack>>);
    impl Drop for Dropper {
        fn drop(&mut self) {
            STACK_PTR.with(|x| x.set(self.0))
        }
    }
    let _dropper = Dropper(ptr);
    f()
}

pub(crate) fn with_stack_context<F, R>(f: F) -> R
where
    F: FnOnce(&Stack) -> R,
{
    let ptr = STACK_PTR
        .with(|x| x.get())
        .expect("Not within a stack context");
    unsafe { f(ptr.as_ref()) }
}

/// A small minimal runtime for executing futures flattened onto the heap preventing stack
/// overflows on deeply nested futures. Only capable of running a single future at the same time
/// and has no support for waking tasks by itself.
pub struct Stack {
    state: Cell<State>,
    tasks: StackTasks,
    context: Cell<Owned<()>>,
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
            tasks: StackTasks::new(),
            context: Cell::new(Owned::dangling()),
        }
    }

    /// Create a new empty stack to run reblessive futures in with atleast cap bytes reserved for
    /// future allocation.
    pub fn with_capacity(cap: usize) -> Self {
        Stack {
            state: Cell::new(State::Base),
            tasks: StackTasks::with_capacity(cap),
            context: Cell::new(Owned::dangling()),
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

            let place: Box<UnsafeCell<Option<R>>> = Box::new(UnsafeCell::new(None));
            let place_ptr = Owned::from_ptr_unchecked(Box::into_raw(place));
            let fut = (f)(ctx);

            self.tasks.borrow_mut().push(async move {
                place_ptr.as_ref().get().write(Some(fut.await));
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
        let this = Defer::new(self, |this| {
            // Ensure that if the task panics it is being dropped
            unsafe { this.tasks.pop() };
        });
        let Some(mut task) = this.tasks.last() else {
            panic!("Missing tasks");
        };

        match unsafe { task.drive(cx) } {
            Poll::Pending => {
                this.take();
                Poll::Pending
            }
            Poll::Ready(_) => Poll::Ready(()),
        }
    }

    pub(crate) fn push_new_task<F>(&self, f: F)
    where
        F: Future<Output = ()>,
    {
        self.tasks.push(f);
        self.state.set(State::NewTask);
    }

    pub(crate) fn pop_cancelled_task(&self) {
        unsafe { self.tasks.pop() }
    }

    pub(crate) fn pending_tasks_count(&self) -> usize {
        self.tasks.len()
    }

    pub(crate) fn set_context(&self, cx: Owned<()>) -> Owned<()> {
        self.context.replace(cx)
    }

    pub(crate) fn is_root_context(&self, ctx: &mut Context) -> bool {
        self.context.get() == Owned::from(ctx).cast()
    }

    pub(crate) fn clear(&self) {
        let prev = self.state.replace(State::Cancelled);
        enter_stack_context(self, || self.tasks.clear());
        self.state.set(prev);
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        self.clear()
    }
}
