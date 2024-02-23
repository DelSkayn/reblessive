mod allocator;
mod ctx;
mod stub_ctx;
mod task;

#[cfg(test)]
mod test;

use std::{
    cell::UnsafeCell,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

pub use ctx::Ctx;
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
            loop {
                let tasks = &this.runner.ptr.as_ref().0;
                let tasks_len = tasks.len();
                let Some(mut task) = tasks.head() else {
                    panic!("Tasks empty")
                };

                match task.drive(cx) {
                    Poll::Pending => {
                        if tasks.len() > tasks_len {
                            continue;
                        }
                        return Poll::Pending;
                    }
                    Poll::Ready(_) => {
                        tasks.drop(task);
                        if tasks_len == 1 {
                            let value = (*this.runner.place.as_ref().get()).take().unwrap();
                            return Poll::Ready(value);
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
            let tasks = &this.runner.ptr.as_ref().0;
            let tasks_len = tasks.len();
            let Some(mut task) = tasks.head() else {
                panic!("Tasks empty")
            };

            match task.drive(cx) {
                Poll::Pending => {
                    if tasks.len() == tasks_len {
                        return Poll::Pending;
                    }
                }
                Poll::Ready(_) => {
                    tasks.drop(task);
                    if tasks_len == 1 {
                        let value = (*this.runner.place.as_ref().get()).take().unwrap();
                        return Poll::Ready(Some(value));
                    }
                }
            }
            Poll::Ready(None)
        }
    }
}

pub struct Runner<'a, R> {
    place: NonNull<UnsafeCell<Option<R>>>,
    ptr: NonNull<Stack>,
    _stack_marker: PhantomData<&'a mut Stack>,
    _res_marker: PhantomData<R>,
}

impl<'a, R> Runner<'a, R> {
    pub fn finish(mut self) -> R {
        loop {
            if let Some(x) = self.step() {
                return x;
            }
        }
    }

    pub fn step(&mut self) -> Option<R> {
        unsafe {
            {
                let Some(mut task) = self.ptr.as_ref().0.head() else {
                    return Some((*self.place.as_ref().get()).take().unwrap());
                };

                let waker = stub_ctx::get();
                let mut context = Context::from_waker(&waker);

                match task.drive(&mut context) {
                    Poll::Pending => return None,
                    Poll::Ready(_) => self.ptr.as_ref().0.drop(task),
                }
            }
        }
        None
    }

    pub fn step_async<'b>(&'b mut self) -> StepFuture<'b, 'a, R> {
        StepFuture { runner: self }
    }

    pub fn depth(&self) -> usize {
        unsafe { self.ptr.as_ref().0.len() }
    }

    pub fn finish_async(self) -> RunnerFuture<'a, R> {
        RunnerFuture { runner: self }
    }
}

impl<'a, R> Drop for Runner<'a, R> {
    fn drop(&mut self) {
        unsafe {
            let stack = self.ptr.as_ref();
            while let Some(t) = stack.0.head() {
                stack.0.drop(t)
            }
            let _ = Box::from_raw(self.place.as_ptr());
        }
    }
}

/// A small minimal runtime for executing futures flattened onto the heap preventing stack
/// overflows on deeply nested futures. Only capable of running a single future at the same time
/// and has no support for waking tasks by itself.
pub struct Stack(Tasks);

impl Stack {
    /// Create a new empty stack to run reblessive futures in.
    ///
    /// This function does not allocate.
    pub fn new() -> Self {
        Stack(Tasks::new())
    }

    /// Create a new empty stack to run reblessive futures in with atleast cap bytes reserved for
    /// future allocation.
    pub fn with_capacity(cap: usize) -> Self {
        Stack(Tasks::with_capacity(cap))
    }

    /// Run a future in the stack.
    pub fn run<'a, F, Fut, R>(&'a mut self, f: F) -> Runner<'a, R>
    where
        F: FnOnce(Ctx<'a>) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        unsafe {
            let ctx = Ctx::new_ptr(NonNull::from(&*self));

            let place = Box::new(UnsafeCell::new(None));
            let place_ptr = NonNull::new_unchecked(Box::into_raw(place));

            self.0.push(TaskFuture {
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
