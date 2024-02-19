mod allocator;
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

use allocator::StackAllocator;
use pin_project_lite::pin_project;
use task::Task;

pin_project_lite::pin_project! {
    pub struct CtxFuture<'a, 's, F, R> {
        ptr: NonNull<Stack>,
        f: Option<F>,
        res: UnsafeCell<Option<R>>,
        _marker: PhantomData<&'a mut Ctx<'s>>,
    }
}

impl<'a, 's, F, Fut, R> Future for CtxFuture<'a, 's, F, R>
where
    F: FnOnce(Ctx<'s>) -> Fut,
    Fut: Future<Output = R> + 's,
{
    type Output = R;

    #[inline]
    fn poll(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        unsafe {
            if let Some(x) = this.f.take() {
                let ctx = Ctx::new_ptr(*this.ptr);

                let future_ptr = (*this.ptr.as_ref().allocator.get()).alloc::<TaskFuture<Fut, R>>();

                future_ptr.as_ptr().write(TaskFuture {
                    place: NonNull::from(this.res),
                    inner: (x)(ctx),
                });

                (*this.ptr.as_ref().tasks.get()).push(Task::wrap_future(future_ptr));
                return Poll::Pending;
            }

            if let Some(x) = (*this.res.get()).take() {
                return Poll::Ready(x);
            }
        }
        Poll::Pending
    }
}

pin_project! {
    struct TaskFuture<F, R> {
        place: NonNull<UnsafeCell<Option<R>>>,
        #[pin]
        inner: F,
    }
}

impl<F, R> Future for TaskFuture<F, R>
where
    F: Future<Output = R>,
{
    type Output = ();

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let this = self.project();
        unsafe {
            let res = std::task::ready!(this.inner.poll(cx));
            this.place.as_ref().get().write(Some(res));
            Poll::Ready(())
        }
    }
}

pub struct Ctx<'s> {
    ptr: NonNull<Stack>,
    _marker: PhantomData<&'s Stack>,
}

impl<'s> Ctx<'s> {
    unsafe fn new_ptr(ptr: NonNull<Stack>) -> Self {
        Ctx {
            ptr,
            _marker: PhantomData,
        }
    }
}

unsafe impl<'s> Send for Ctx<'s> {}
unsafe impl<'s> Sync for Ctx<'s> {}

impl<'s> Ctx<'s> {
    pub fn run<'a, F, Fut, R>(&'a mut self, f: F) -> CtxFuture<'a, 's, F, R>
    where
        F: FnOnce(Ctx<'s>) -> Fut,
        Fut: Future<Output = R> + 's,
    {
        CtxFuture {
            ptr: self.ptr,
            f: Some(f),
            res: UnsafeCell::new(None),
            _marker: PhantomData,
        }
    }
}

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
                let tasks = &this.runner.ptr.as_ref().tasks;
                let tasks_len = (*tasks.get()).len();
                let Some(x) = (*tasks.get()).last_mut() else {
                    panic!("Tasks empty")
                };

                match x.drive(cx) {
                    Poll::Pending => {
                        if (*tasks.get()).len() > tasks_len {
                            continue;
                        }
                        return Poll::Pending;
                    }
                    Poll::Ready(_) => {
                        let task = (*tasks.get()).pop().unwrap();
                        task.drop();
                        (*this.runner.ptr.as_ref().allocator.get()).pop_deallocate();
                        if (*tasks.get()).is_empty() {
                            let value = (*this.runner.place.as_ref().get()).take().unwrap();
                            return Poll::Ready(value);
                        }
                    }
                }
            }
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
                let Some(mut x) = (*self.ptr.as_ref().tasks.get()).last().cloned() else {
                    panic!("Tasks empty")
                };
                let waker = stub_ctx::get();
                let mut context = Context::from_waker(&waker);

                match x.drive(&mut context) {
                    Poll::Pending => return None,
                    Poll::Ready(_) => {}
                }
            }

            let task = (*self.ptr.as_ref().tasks.get()).pop().unwrap();
            task.drop();
            (*self.ptr.as_ref().allocator.get()).pop_deallocate();
            if (*self.ptr.as_ref().tasks.get()).is_empty() {
                return Some((*self.place.as_ref().get()).take().unwrap());
            }
        }
        None
    }

    pub fn depth(&self) -> usize {
        unsafe { (*self.ptr.as_ref().tasks.get()).len() }
    }

    pub fn finish_async(self) -> RunnerFuture<'a, R> {
        RunnerFuture { runner: self }
    }
}

impl<'a, R> Drop for Runner<'a, R> {
    fn drop(&mut self) {
        unsafe {
            let _ = Box::from_raw(self.place.as_ptr());
            let stack = self.ptr.as_ref();
            while let Some(t) = (*stack.tasks.get()).pop() {
                t.drop();
                (*stack.allocator.get()).pop_deallocate();
            }
        }
    }
}

pub struct Stack {
    allocator: UnsafeCell<StackAllocator>,
    tasks: UnsafeCell<Vec<Task>>,
}

impl Stack {
    pub fn new() -> Self {
        Stack {
            allocator: UnsafeCell::new(StackAllocator::new()),
            tasks: UnsafeCell::new(Vec::new()),
        }
    }

    pub fn run<'a, F, Fut, R>(&'a mut self, f: F) -> Runner<'a, R>
    where
        F: FnOnce(Ctx<'a>) -> Fut,
        Fut: Future<Output = R> + 'a,
    {
        unsafe {
            let ctx = Ctx::new_ptr(NonNull::from(&*self));

            let place = Box::new(UnsafeCell::new(None));
            let place_ptr = NonNull::new_unchecked(Box::into_raw(place));

            let future_ptr = (*self.allocator.get()).alloc::<TaskFuture<Fut, R>>();

            future_ptr.as_ptr().write(TaskFuture {
                place: place_ptr,
                inner: (f)(ctx),
            });

            (*self.tasks.get()).push(Task::wrap_future(future_ptr));

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
