//! The stack runtime
//!
//! A runtime for turning recursive functions into a number of futures which are run from a single
//! flattened loop, preventing stack overflows.
//!
//! This runtime also has support for external async function but it explicitly doesn't support
//! intra-task concurrency, i.e. calling select or join on multiple futures at the same time. These
//! types of patterns break the stack allocation pattern which this executor uses to be able to
//! allocate and run futures efficiently.

use crate::{
    allocator::StackAllocator,
    defer::Defer,
    ptr::{map_ptr, Owned},
    vtable::{TaskBox, VTable},
};
use std::{
    cell::{Cell, UnsafeCell},
    future::Future,
    marker::PhantomData,
    task::{Context, Poll},
};

pub(crate) mod future;
mod runner;
mod stk;
#[cfg(test)]
mod test;

pub use future::{StkFuture, YieldFuture};
pub use runner::{FinishFuture, Runner, StepFuture};
pub(crate) use stk::StackMarker;
pub use stk::Stk;

thread_local! {
    static STACK_PTR: Cell<Option<Owned<Stack>>> = const { Cell::new(None) };
}

type ResultPlace<T> = UnsafeCell<Option<T>>;

#[derive(Clone, Copy, Eq, PartialEq, Debug)]
pub(crate) enum StackState {
    /// Initial stack state
    Base,
    /// A future requested a that the executor yield
    Yield,
    /// The pending tasks in the executor are being canceled.  
    Cancelled,
    /// A new task was created, execution should switch to the new task
    NewTask,
}

/// A small minimal runtime for executing futures flattened onto the heap preventing stack
/// overflows on deeply nested futures.
///
/// Only capable of running a single future at the same time and has no support for waking
/// tasks by itself.
pub struct Stack {
    allocator: UnsafeCell<StackAllocator>,
    pub(crate) len: Cell<usize>,
    pub(crate) state: Cell<StackState>,
    async_context: Cell<usize>,
}

unsafe impl Send for Stack {}
unsafe impl Sync for Stack {}

impl Stack {
    /// Create a new empty stack to run reblessive futures in.
    ///
    /// This function does not allocate.
    pub fn new() -> Self {
        Self {
            allocator: UnsafeCell::new(StackAllocator::new()),
            len: Cell::new(0),
            state: Cell::new(StackState::Base),
            async_context: Cell::new(Owned::<u8>::dangling().addr().get()),
        }
    }

    /// Run a future in the stack.
    pub fn enter<'a, F, Fut, R>(&'a mut self, f: F) -> Runner<'a, R>
    where
        F: FnOnce(&'a mut Stk) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        let fut = unsafe { f(Stk::create()) };

        unsafe { self.enter_future(fut) }

        Runner {
            stack: self,
            _marker: PhantomData,
        }
    }

    pub(crate) unsafe fn enter_future<F, R>(&self, fut: F)
    where
        F: Future<Output = R>,
    {
        assert_eq!(
            self.len.get(),
            0,
            "Stack still has unresolved futures, did a previous runner leak?"
        );

        let place_ptr =
            (*self.allocator.get()).alloc_with::<ResultPlace<R>, _>(|| UnsafeCell::new(None));

        self.len.set(1);

        self.alloc_future(
            async move { unsafe { place_ptr.as_ref().get().write(Some(fut.await)) } },
        );
    }

    pub(crate) fn enter_context<F, R>(&self, f: F) -> R
    where
        F: FnOnce() -> R,
    {
        let ptr = STACK_PTR.with(|x| x.replace(Some(Owned::from(self))));
        let _defer = Defer::new(ptr, |ptr| {
            let ptr = *ptr;
            STACK_PTR.with(|x| x.set(ptr));
        });
        f()
    }

    pub(crate) fn with_context<F, R>(f: F) -> R
    where
        F: FnOnce(&Self) -> R,
    {
        let ptr = STACK_PTR
            .with(|x| x.get())
            .expect("Not within a stack context");
        unsafe { f(ptr.as_ref()) }
    }

    unsafe fn alloc_future<F>(&self, f: F) -> Owned<TaskBox<u8>>
    where
        F: Future<Output = ()>,
    {
        let res = (*self.allocator.get())
            .alloc_with(|| TaskBox {
                v_table: VTable::get::<F>(),
                future: f,
            })
            .cast();
        self.len.set(self.len.get() + 1);
        res
    }

    pub(crate) fn pending_tasks(&self) -> usize {
        self.len.get().saturating_sub(1)
    }

    // tries to get the final result of the last allocation
    pub(crate) unsafe fn try_get_result<R>(&self) -> Option<R> {
        if self.len.get() == 1 {
            let place_ptr = (*self.allocator.get()).top().unwrap();
            let res = (*place_ptr.cast::<ResultPlace<R>>().as_ref().get()).take();
            assert!(
                res.is_some(),
                "Result was not writen even after all futures finished!",
            );

            (*self.allocator.get()).pop_dealloc();
            self.len.set(0);

            return res;
        }
        None
    }

    pub(crate) unsafe fn drive_top_task(&self, context: &mut Context) -> Poll<bool> {
        self.enter_context(|| {
            let task = (*self.allocator.get()).top().unwrap().cast::<TaskBox<u8>>();
            let r = Self::drive_task(task, context);
            match r {
                Poll::Ready(_) => {
                    // task was ready, it can be popped.
                    self.pop_task();
                    Poll::Ready(false)
                }
                Poll::Pending => {
                    // the future yielded, find out why,
                    match self.state.get() {
                        StackState::Base => {
                            // State didn't change, but future still yielded,
                            // This means that some outside future yielded so we should yield too.
                            Poll::Pending
                        }
                        StackState::Yield => {
                            // A future requested a yield for the reblessive runtime
                            self.state.set(StackState::Base);
                            Poll::Ready(true)
                        }
                        StackState::Cancelled => {
                            panic!("stack should never be running tasks while cancelling")
                        }
                        StackState::NewTask => {
                            self.state.set(StackState::Base);
                            Poll::Ready(false)
                        }
                    }
                }
            }
        })
    }

    pub(crate) unsafe fn push_task<F>(&self, f: F)
    where
        F: Future<Output = ()>,
    {
        let old = self.state.replace(StackState::NewTask);
        assert_eq!(
            old,
            StackState::Base,
            "Invalid stack state, futures are not being evaluated in stack order."
        );
        self.alloc_future(f);
    }

    pub(crate) unsafe fn pop_task(&self) {
        let task = (*self.allocator.get()).top().unwrap().cast::<TaskBox<u8>>();
        Self::drop_task_inline(task);
        (*self.allocator.get()).pop_dealloc();
        self.len.set(self.len.get() - 1);
    }

    pub(crate) unsafe fn pop_cancel_task(&self) {
        let old_state = self.state.replace(StackState::Cancelled);
        self.pop_task();
        self.state.set(old_state);
    }

    unsafe fn drive_task(drive: Owned<TaskBox<u8>>, context: &mut Context) -> Poll<()> {
        let v_table = drive.map_ptr(map_ptr!(Owned<TaskBox<u8>>, v_table)).read();
        (v_table.driver)(drive, context)
    }

    unsafe fn drop_task_inline(drive: Owned<TaskBox<u8>>) {
        let v_table = drive.map_ptr(map_ptr!(Owned<TaskBox<u8>>, v_table)).read();
        (v_table.dropper)(drive)
    }

    pub(crate) fn is_rebless_context(&self, context: &mut Context) -> bool {
        self.async_context.get() == Owned::from(context).addr().get()
    }

    pub(crate) fn set_rebless_context(&self, context: &mut Context) -> usize {
        self.set_rebless_context_addr(Owned::from(context).addr().get())
    }

    pub(crate) fn set_rebless_context_addr(&self, context: usize) -> usize {
        self.async_context.replace(context)
    }

    pub(crate) unsafe fn clear<R>(&self) {
        let len = self.len.get();
        if len == 0 {
            // No tasks pushed, nothing to be done.
            return;
        }

        self.state.set(StackState::Cancelled);

        // Clear all the futures
        self.enter_context(|| {
            let borrow = &mut (*self.allocator.get());
            for _ in (1..len).rev() {
                unsafe { Self::drop_task_inline(borrow.top().unwrap().cast()) }
                borrow.pop_dealloc();
            }
        });

        // If the count was less then or equal to 2 it means that the final future might have
        // produced a result, so we need to take the possible result out of final pointer to run
        // it's drop, if any
        if std::mem::needs_drop::<R>() && len <= 2 {
            let place_ptr = (*self.allocator.get()).top().unwrap();
            (*place_ptr.cast::<UnsafeCell<Option<R>>>().as_ref().get()).take();
        }

        // Deallocate the result allocation.
        (*self.allocator.get()).pop_dealloc();
        // reset len since the stack is now empty.
        self.len.set(0);
        self.state.set(StackState::Base);
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for Stack {
    fn drop(&mut self) {
        // clear some memory, this might leak if the root value had allocations but this leak will
        // only happen if the Runner was leaked.
        unsafe { self.clear::<()>() }
    }
}
