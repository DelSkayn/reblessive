use crate::{allocator::StackAllocator, vtable::VTable};
use std::{
    alloc::Layout,
    cell::{Cell, UnsafeCell},
    future::Future,
    marker::PhantomData,
    ptr::{addr_of_mut, NonNull},
    task::{Context, Poll},
};

#[repr(C)]
struct TaskBox<F> {
    header: TaskBoxHeader,
    future: F,
}

struct TaskBoxHeader {
    v_table: &'static VTable,
    previous: Option<NonNull<TaskBox<u8>>>,
}

pub struct Task<'a> {
    ptr: NonNull<TaskBox<u8>>,
    _marker: PhantomData<&'a TaskBox<u8>>,
}

impl Task<'_> {
    pub unsafe fn drive(&mut self, ctx: &mut Context<'_>) -> Poll<()> {
        let ptr_off = addr_of_mut!((*self.ptr.as_ptr()).future);
        (self.ptr.as_ref().header.v_table.driver)(NonNull::new_unchecked(ptr_off), ctx)
    }
}

/// A stack for tasks.
pub struct StackTasks {
    last: Cell<Option<NonNull<TaskBox<u8>>>>,
    allocator: UnsafeCell<StackAllocator>,
    len: Cell<usize>,
}

impl StackTasks {
    pub fn new() -> Self {
        StackTasks {
            last: Cell::new(None),
            allocator: UnsafeCell::new(StackAllocator::new()),
            len: Cell::new(0),
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        StackTasks {
            last: Cell::new(None),
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
            let ptr = (*self.allocator.get())
                .push_alloc(Layout::new::<TaskBox<F>>())
                .cast::<TaskBox<F>>();
            let v_table = VTable::get::<F>();
            ptr.as_ptr().write(TaskBox {
                header: TaskBoxHeader {
                    v_table,
                    previous: self.last.get(),
                },
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

    unsafe fn drop_in_place(ptr: NonNull<TaskBox<u8>>) {
        let future_ptr = addr_of_mut!((*ptr.as_ptr()).future);
        (ptr.as_ref().header.v_table.dropper)(NonNull::new_unchecked(future_ptr))
    }

    pub unsafe fn pop(&self) {
        let len = self.len();
        debug_assert_ne!(len, 0);
        self.len.set(len - 1);
        let last = self.last.get().unwrap();
        self.last.set(last.as_ref().header.previous);
        let layout = last.as_ref().header.v_table.layout;
        let layout = Layout::new::<TaskBoxHeader>()
            .extend(layout)
            .unwrap()
            .0
            .pad_to_align();

        Self::drop_in_place(last);
        (*self.allocator.get()).pop_dealloc(layout);
    }

    pub fn clear(&self) {
        for _ in 0..self.len() {
            unsafe { self.pop() }
        }
    }
}
