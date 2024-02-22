use std::{
    cell::UnsafeCell,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    ptr::{addr_of_mut, NonNull},
    task::{Context, Poll},
};

use crate::allocator::StackAllocator;

#[derive(Debug, Clone)]
pub struct TaskFunctions {
    dropper: unsafe fn(NonNull<()>),
    driver: unsafe fn(NonNull<()>, ctx: &mut Context<'_>) -> Poll<()>,
}

#[repr(C)]
struct InnerStack<F> {
    task: TaskFunctions,
    future: F,
}

pub struct Task<'a> {
    ptr: NonNull<InnerStack<()>>,
    _marker: PhantomData<&'a InnerStack<()>>,
}

impl Task<'_> {
    pub fn drive(&mut self, ctx: &mut Context<'_>) -> Poll<()> {
        unsafe {
            let future_ptr = NonNull::new_unchecked(addr_of_mut!((*self.ptr.as_ptr()).future));
            (self.ptr.as_ref().task.driver)(future_ptr, ctx)
        }
    }

    fn drop_in_place(&mut self) {
        unsafe {
            let future_ptr = NonNull::new_unchecked(addr_of_mut!((*self.ptr.as_ptr()).future));
            (self.ptr.as_ref().task.dropper)(future_ptr)
        }
    }
}

pub struct Tasks(UnsafeCell<StackAllocator>);

impl Tasks {
    pub fn new() -> Self {
        Tasks(UnsafeCell::new(StackAllocator::new()))
    }

    pub fn with_capacity(cap: usize) -> Self {
        Tasks(UnsafeCell::new(StackAllocator::with_capacity(cap)))
    }

    pub fn len(&self) -> usize {
        unsafe { (*self.0.get()).allocations() }
    }

    pub fn push<F>(&self, f: F)
    where
        F: Future<Output = ()>,
    {
        unsafe {
            let ptr = (*self.0.get()).alloc::<InnerStack<F>>();
            ptr.as_ptr().write(InnerStack {
                task: TaskFunctions {
                    dropper: Self::drop_impl::<F>,
                    driver: Self::drive_impl::<F>,
                },
                future: f,
            })
        }
    }

    pub fn head(&self) -> Option<Task> {
        unsafe {
            Some(Task {
                ptr: (*self.0.get()).head()?.cast(),
                _marker: PhantomData,
            })
        }
    }

    pub unsafe fn drop(&self, mut task: Task<'_>) {
        task.drop_in_place();
        (*self.0.get()).pop_deallocate()
    }

    unsafe fn drop_impl<F>(ptr: NonNull<()>)
    where
        F: Future<Output = ()>,
    {
        std::ptr::drop_in_place(ptr.cast::<F>().as_ptr())
    }

    unsafe fn drive_impl<F>(ptr: NonNull<()>, ctx: &mut Context<'_>) -> Poll<()>
    where
        F: Future<Output = ()>,
    {
        Pin::new_unchecked(ptr.cast::<F>().as_mut()).poll(ctx)
    }
}
