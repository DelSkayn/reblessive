mod allocator;
mod stub_ctx;
mod task;

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

/// A marker struct which marks a lifetime as invariant.
///
// Since 'inv required to be both contravariant as well as covariant the result is an invariant
// lifetime.
#[derive(Copy, Clone, Eq, PartialEq, PartialOrd, Ord, Hash, Debug, Default)]
pub struct Invariant<'inv>(PhantomData<&'inv mut &'inv fn(&'inv ()) -> &'inv ()>);

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
                let future_ptr = this.ptr.as_mut().allocator.alloc::<TaskFuture<Fut, R>>();

                future_ptr.as_ptr().write(TaskFuture {
                    place: NonNull::from(this.res),
                    inner: (x)(ctx),
                });

                this.ptr.as_mut().tasks.push(Task::wrap_future(future_ptr));
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
                let tasks_len = this.runner.ptr.as_ref().tasks.len();
                let Some(x) = this.runner.ptr.as_mut().tasks.last_mut() else {
                    panic!("Tasks empty")
                };

                match x.drive(cx) {
                    Poll::Pending => {
                        if this.runner.ptr.as_ref().tasks.len() > tasks_len {
                            continue;
                        }
                        return Poll::Pending;
                    }
                    Poll::Ready(_) => {
                        let task = this.runner.ptr.as_mut().tasks.pop().unwrap();
                        task.drop();
                        this.runner.ptr.as_mut().allocator.pop_deallocate();
                        if this.runner.ptr.as_ref().tasks.is_empty() {
                            let value = (*this.runner.place.get()).take().unwrap();
                            return Poll::Ready(value);
                        }
                    }
                }
            }
        }
    }
}

pub struct Runner<'a, R> {
    place: Box<UnsafeCell<Option<R>>>,
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
                let Some(x) = self.ptr.as_mut().tasks.last_mut() else {
                    panic!("Tasks empty")
                };
                let waker = stub_ctx::get();
                let mut context = Context::from_waker(&waker);

                match x.drive(&mut context) {
                    Poll::Pending => return None,
                    Poll::Ready(_) => {}
                }
            }

            let task = self.ptr.as_mut().tasks.pop().unwrap();
            task.drop();
            self.ptr.as_mut().allocator.pop_deallocate();
            if self.ptr.as_ref().tasks.is_empty() {
                return Some((*self.place.get()).take().unwrap());
            }
        }
        None
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
            let stack = self.ptr.as_mut();
            while let Some(t) = stack.tasks.pop() {
                t.drop();
                stack.allocator.pop_deallocate();
            }
        }
    }
}

pub struct Stack {
    allocator: StackAllocator,
    tasks: Vec<Task>,
}

impl Stack {
    pub fn new() -> Self {
        Stack {
            allocator: StackAllocator::new(),
            tasks: Vec::new(),
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
            let place_ptr = NonNull::from(place.as_ref());

            let future_ptr = self.allocator.alloc::<TaskFuture<Fut, R>>();

            future_ptr.as_ptr().write(TaskFuture {
                place: place_ptr,
                inner: (f)(ctx),
            });

            self.tasks.push(Task::wrap_future(future_ptr));

            Runner {
                place,
                ptr: NonNull::from(self),
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
