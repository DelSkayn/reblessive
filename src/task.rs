use std::{
    alloc::Layout,
    cell::{Cell, UnsafeCell},
    future::Future,
    marker::PhantomData,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

use crate::allocator::StackAllocator;

/// A constant table generated for each type of tasks that is spawned.
#[derive(Debug, Clone)]
pub struct TaskVTable {
    /// Funtion to drop the task in place.
    dropper: unsafe fn(NonNull<AllocatedTask<u8>>),
    /// Funtion to drive the task forward.
    driver: unsafe fn(NonNull<AllocatedTask<u8>>, ctx: &mut Context<'_>) -> Poll<()>,
    /// The allocation layout.
    alloc_layout: Layout,
}

#[repr(C)]
struct AllocatedTask<F> {
    v_table: &'static TaskVTable,
    previous: Option<NonNull<AllocatedTask<u8>>>,
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
                alloc_layout: Layout::new::<AllocatedTask<Self>>(),
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
    pub unsafe fn drive(&mut self, ctx: &mut Context<'_>) -> Poll<()> {
        (self.ptr.as_ref().v_table.driver)(self.ptr, ctx)
    }
}

pub struct Tasks {
    last: Cell<Option<NonNull<AllocatedTask<u8>>>>,
    allocator: UnsafeCell<StackAllocator>,
    len: Cell<usize>,
}

impl Tasks {
    pub fn new() -> Self {
        Tasks {
            last: Cell::new(None),
            allocator: UnsafeCell::new(StackAllocator::new()),
            len: Cell::new(0),
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        Tasks {
            last: Cell::new(None),
            allocator: UnsafeCell::new(StackAllocator::with_capacity(cap)),
            len: Cell::new(0),
        }
    }

    pub fn len(&self) -> usize {
        self.len.get()
    }

    pub fn push<F>(&self, f: F)
    where
        F: Future<Output = ()>,
    {
        unsafe {
            let ptr = (*self.allocator.get())
                .push_alloc(Layout::new::<AllocatedTask<F>>())
                .cast::<AllocatedTask<F>>();
            let v_table = TaskVTable::get::<F>();
            ptr.as_ptr().write(AllocatedTask {
                v_table,
                previous: self.last.get(),
                future: f,
            });
            self.last.set(Some(ptr.cast()));
            self.len.set(self.len.get() + 1)
        }
    }

    pub fn last(&self) -> Option<Task> {
        Some(Task {
            ptr: self.last.get()?,
            _marker: PhantomData,
        })
    }

    unsafe fn drop_in_place(ptr: NonNull<AllocatedTask<u8>>) {
        (ptr.as_ref().v_table.dropper)(ptr)
    }

    pub unsafe fn pop(&self) {
        let len = self.len();
        debug_assert_ne!(len, 0);
        self.len.set(len - 1);
        let last = self.last.get().unwrap();
        self.last.set(last.as_ref().previous);
        let layout = last.as_ref().v_table.alloc_layout;
        Self::drop_in_place(last);
        (*self.allocator.get()).pop_dealloc(layout);
    }

    pub fn clear(&self) {
        for _ in 0..self.len() {
            unsafe { self.pop() }
        }
    }
}

impl Drop for Tasks {
    fn drop(&mut self) {
        self.clear()
    }
}
