use std::{
    cell::UnsafeCell,
    future::Future,
    marker::PhantomData,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

use crate::allocator::StackAllocator;

#[derive(Debug, Clone)]
#[repr(align(16))]
pub struct TaskVTable {
    dropper: unsafe fn(NonNull<AllocatedTask<u8>>),
    driver: unsafe fn(NonNull<AllocatedTask<u8>>, ctx: &mut Context<'_>) -> Poll<()>,
    //alloc_layout: Layout,
}

#[repr(C)]
struct AllocatedTask<F> {
    v_table: &'static TaskVTable,
    future: F,
}

impl TaskVTable {
    pub fn get<F: Future<Output = ()>>() -> &'static TaskVTable {
        trait HasVTable {
            const V_TABLE: TaskVTable;
        }

        impl<F: Future<Output = ()>> HasVTable for F {
            const V_TABLE: TaskVTable = TaskVTable {
                dropper: TaskVTable::drop_impl::<F>,
                driver: TaskVTable::drive_impl::<F>,
                //alloc_layout: Layout::new::<AllocatedTask<Self>>(),
            };
        }

        &<F as HasVTable>::V_TABLE
    }

    unsafe fn drop_impl<F>(ptr: NonNull<AllocatedTask<u8>>)
    where
        F: Future<Output = ()>,
    {
        std::ptr::drop_in_place(ptr.cast::<AllocatedTask<F>>().as_ptr())
    }

    unsafe fn drive_impl<F>(ptr: NonNull<AllocatedTask<u8>>, ctx: &mut Context<'_>) -> Poll<()>
    where
        F: Future<Output = ()>,
    {
        Pin::new_unchecked(&mut ptr.cast::<AllocatedTask<F>>().as_mut().future).poll(ctx)
    }
}

pub struct Task<'a> {
    ptr: NonNull<AllocatedTask<u8>>,
    _marker: PhantomData<&'a AllocatedTask<u8>>,
}

impl Task<'_> {
    pub fn drive(&mut self, ctx: &mut Context<'_>) -> Poll<()> {
        unsafe { (self.ptr.as_ref().v_table.driver)(self.ptr, ctx) }
    }

    fn drop_in_place(&mut self) {
        unsafe { (self.ptr.as_ref().v_table.dropper)(self.ptr) }
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
            let ptr = (*self.0.get()).alloc::<AllocatedTask<F>>();
            let v_table = TaskVTable::get::<F>();
            ptr.as_ptr().write(AllocatedTask { v_table, future: f })
        }
    }

    pub fn head(&self) -> Option<Task> {
        unsafe {
            let head = (*self.0.get()).head()?;

            Some(Task {
                ptr: head.cast(),
                _marker: PhantomData,
            })
        }
    }

    pub unsafe fn drop(&self, mut task: Task<'_>) {
        task.drop_in_place();
        (*self.0.get()).pop_deallocate()
    }
}
