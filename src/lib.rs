mod allocator;
mod ctx;
mod stub_ctx;
mod task;

#[cfg(test)]
mod test;

use std::{
    cell::{Cell, UnsafeCell},
    future::Future,
    marker::PhantomData,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

pub use ctx::Stk;
use ctx::TaskFuture;
use pin_project_lite::pin_project;
use task::Tasks;

pin_project! {
    pub struct RunnerFuture<'a,R>{
        runner: Runner<'a,R>
    }
}

impl<'a, R> Future for RunnerFuture<'a, R> {
    type Output = R;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        unsafe {
            let tasks = &this.runner.ptr.as_ref().tasks;
            loop {
                let Some(mut task) = tasks.head() else {
                    panic!("Tasks empty")
                };

                loop {
                    match task.drive(cx) {
                        Poll::Pending => match this.runner.stack_state() {
                            State::Empty => unreachable!(),
                            State::Running => return Poll::Pending,
                            State::NewTask => {
                                this.runner.set_stack_state(State::Running);
                                break;
                            }
                            State::Yield => {
                                this.runner.set_stack_state(State::Running);
                            }
                        },
                        Poll::Ready(_) => {
                            tasks.drop(task);
                            if tasks.len() == 0 {
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
    pub struct StepFuture<'a,'b,R>{
        runner: &'a mut Runner<'b,R>
    }
}

impl<'a, 'b, R> Future for StepFuture<'a, 'b, R> {
    type Output = Option<R>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();

        unsafe {
            let Some(mut task) = this.runner.ptr.as_ref().tasks.head() else {
                panic!("tasks already empty");
            };

            match task.drive(cx) {
                Poll::Pending => match this.runner.stack_state() {
                    State::Empty => unreachable!(),
                    State::Yield => {
                        this.runner.set_stack_state(State::Running);
                    }
                    State::Running => return Poll::Pending,
                    State::NewTask => {
                        this.runner.set_stack_state(State::Running);
                    }
                },
                Poll::Ready(_) => {
                    this.runner.ptr.as_ref().tasks.drop(task);
                    if this.runner.ptr.as_ref().tasks.len() == 0 {
                        return Poll::Ready(Some(
                            (*this.runner.place.as_ref().get()).take().unwrap(),
                        ));
                    }
                }
            }
        }
        Poll::Ready(None)
    }
}

pub struct Runner<'a, R> {
    place: NonNull<UnsafeCell<Option<R>>>,
    ptr: NonNull<Stack>,
    _stack_marker: PhantomData<&'a mut Stack>,
    _res_marker: PhantomData<R>,
}

unsafe impl<'a, R> Send for Runner<'a, R> {}
unsafe impl<'a, R> Sync for Runner<'a, R> {}

impl<'a, R> Runner<'a, R> {
    fn stack_state(&self) -> State {
        unsafe { self.ptr.as_ref().state.get() }
    }

    fn set_stack_state(&self, state: State) {
        unsafe { self.ptr.as_ref().state.set(state) }
    }

    pub fn finish(mut self) -> R {
        unsafe { self.finish_inner() }
    }

    unsafe fn finish_inner(&mut self) -> R {
        while let Some(mut task) = self.ptr.as_ref().tasks.head() {
            let waker = stub_ctx::get();
            let mut context = Context::from_waker(&waker);

            loop {
                match task.drive(&mut context) {
                    Poll::Pending => match self.stack_state() {
                        State::Empty => unreachable!(),
                        State::Yield => {
                            self.set_stack_state(State::Running);
                        }
                        State::Running => {}
                        State::NewTask => {
                            self.set_stack_state(State::Running);
                            break;
                        }
                    },
                    Poll::Ready(_) => {
                        self.ptr.as_ref().tasks.drop(task);
                        break;
                    }
                }
            }
        }
        (*self.place.as_ref().get()).take().unwrap()
    }

    pub fn step(&mut self) -> Option<R> {
        unsafe {
            let Some(mut task) = self.ptr.as_ref().tasks.head() else {
                panic!("tasks already empty");
            };

            let waker = stub_ctx::get();
            let mut context = Context::from_waker(&waker);

            match task.drive(&mut context) {
                Poll::Pending => match self.stack_state() {
                    State::Empty => unreachable!(),
                    State::Yield => {
                        self.set_stack_state(State::Running);
                    }
                    State::Running => {}
                    State::NewTask => {
                        self.set_stack_state(State::Running);
                    }
                },
                Poll::Ready(_) => {
                    self.ptr.as_ref().tasks.drop(task);
                    if self.ptr.as_ref().tasks.len() == 0 {
                        return Some((*self.place.as_ref().get()).take().unwrap());
                    }
                }
            }
        }
        None
    }

    pub fn step_async<'b>(&'b mut self) -> StepFuture<'b, 'a, R> {
        StepFuture { runner: self }
    }

    pub fn depth(&self) -> usize {
        unsafe { self.ptr.as_ref().tasks.len() }
    }

    pub fn finish_async(self) -> RunnerFuture<'a, R> {
        RunnerFuture { runner: self }
    }
}

impl<'a, R> Drop for Runner<'a, R> {
    fn drop(&mut self) {
        unsafe {
            let stack = self.ptr.as_ref();
            while let Some(t) = stack.tasks.head() {
                stack.tasks.drop(t)
            }
            stack.state.set(State::Empty);
            let _ = Box::from_raw(self.place.as_ptr());
        }
    }
}

#[derive(Clone, Copy)]
enum State {
    /// the stack without a future.
    Empty,
    /// normal execution of the stack.
    Running,
    /// A new task was pushed to the Stack
    /// Stack should continue with the newly pushed future.
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
            state: Cell::new(State::Empty),
            tasks: Tasks::new(),
        }
    }

    /// Create a new empty stack to run reblessive futures in with atleast cap bytes reserved for
    /// future allocation.
    pub fn with_capacity(cap: usize) -> Self {
        Stack {
            state: Cell::new(State::Empty),
            tasks: Tasks::with_capacity(cap),
        }
    }

    /// Run a future in the stack.
    pub fn run<'a, F, Fut, R>(&'a mut self, f: F) -> Runner<'a, R>
    where
        F: FnOnce(Stk<'a>) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        self.state.set(State::Running);
        unsafe {
            let ctx = Stk::new_ptr(NonNull::from(&*self));

            let place = Box::new(UnsafeCell::new(None));
            let place_ptr = NonNull::new_unchecked(Box::into_raw(place));

            self.tasks.push(TaskFuture {
                place: place_ptr,
                inner: (f)(ctx),
            });

            Runner {
                place: place_ptr,
                ptr: NonNull::from(&*self),
                _stack_marker: PhantomData,
                _res_marker: PhantomData,
            }
        }
    }
}

impl Default for Stack {
    fn default() -> Self {
        Self::new()
    }
}
