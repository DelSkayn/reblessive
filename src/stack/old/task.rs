use crate::{
    allocator::StackAllocator,
    map_ptr,
    ptr::Mut,
    vtable::{TaskBox, VTable},
};
use std::{
    cell::{Cell, UnsafeCell},
    future::Future,
    io::Write,
    ptr::NonNull,
    task::{Context, Poll},
};

pub struct Task<'a> {
    ptr: Mut<'a, TaskBox<u8>>,
}

impl Task<'_> {
    pub unsafe fn drive(&mut self, ctx: &mut Context<'_>) -> Poll<()> {
        let v_table = self.ptr.as_ref().v_table;
        (v_table.driver)(self.ptr.reborrow().cast(), ctx)
    }
}

/// A stack for tasks.
pub struct StackTasks {
    allocator: UnsafeCell<StackAllocator>,
    len: Cell<usize>,
}

impl StackTasks {
    pub fn new() -> Self {
        StackTasks {
            allocator: UnsafeCell::new(StackAllocator::new()),
            len: Cell::new(0),
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        StackTasks {
            allocator: UnsafeCell::new(StackAllocator::with_capacity(cap)),
            len: Cell::new(0),
        }
    }

    pub fn len(&self) -> usize {
        self.len.get()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn push<F>(&self, f: F)
    where
        F: Future<Output = ()>,
    {
        unsafe {
            let ptr = (*self.allocator.get()).alloc(TaskBox {
                v_table: VTable::get::<F>(),
                future: f,
            });

            let v_table_addr = ptr
                .map_ptr(map_ptr!(TaskBox<F>, v_table))
                .cast::<usize>()
                .read();
            assert_ne!(v_table_addr, 0);

            assert_eq!(Some(ptr), (*self.allocator.get()).top().map(|x| x.cast()));

            self.len.set(self.len.get() + 1)
        }
    }

    pub fn last(&self) -> Option<Task> {
        unsafe {
            let ptr = (*self.allocator.get()).top();
            ptr.map(|x| Task {
                ptr: x.into_mut().cast(),
            })
        }
    }

    unsafe fn drop_in_place(ptr: Mut<TaskBox<u8>>) {
        let v_table = ptr.as_ref().v_table;
        let future_ptr = ptr.map_ptr(map_ptr!(TaskBox<u8>, future));
        (v_table.dropper)(future_ptr)
    }

    pub unsafe fn pop(&self) {
        let len = self.len();
        debug_assert_ne!(len, 0, "Popped more tasks then where pushed.");
        self.len.set(len - 1);

        let last = self.last().unwrap();
        Self::drop_in_place(last.ptr);
        (*self.allocator.get()).pop_dealloc();
    }

    pub fn clear(&self) {
        for _ in 0..self.len() {
            unsafe { self.pop() }
        }
    }
}
