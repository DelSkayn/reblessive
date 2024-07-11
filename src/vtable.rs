use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

use crate::ptr::{map_ptr, Owned};

#[repr(C)]
pub struct TaskBox<F> {
    pub(crate) v_table: &'static VTable,
    pub(crate) future: F,
}

/// A constant table generated for each type of tasks that is spawned.
#[derive(Debug, Clone)]
pub(crate) struct VTable {
    /// Funtion to drop the task in place.
    pub(crate) dropper: unsafe fn(Owned<TaskBox<u8>>),
    /// Funtion to drive the task forward.
    pub(crate) driver: unsafe fn(Owned<TaskBox<u8>>, ctx: &mut Context<'_>) -> Poll<()>,
}

impl VTable {
    pub fn get<F: Future<Output = ()>>() -> &'static VTable {
        trait HasVTable {
            const V_TABLE: VTable;
        }

        impl<F: Future<Output = ()>> HasVTable for F {
            const V_TABLE: VTable = VTable {
                dropper: VTable::drop_impl::<F>,
                driver: VTable::drive_impl::<F>,
            };
        }

        &<F as HasVTable>::V_TABLE
    }

    unsafe fn drop_impl<F>(ptr: Owned<TaskBox<u8>>)
    where
        F: Future<Output = ()>,
    {
        std::ptr::drop_in_place(ptr.cast::<TaskBox<F>>().as_ptr())
    }

    unsafe fn drive_impl<F>(ptr: Owned<TaskBox<u8>>, ctx: &mut Context<'_>) -> Poll<()>
    where
        F: Future<Output = ()>,
    {
        Pin::new_unchecked(
            ptr.cast::<TaskBox<F>>()
                .map_ptr(map_ptr!(TaskBox<F>, future))
                .as_mut(),
        )
        .poll(ctx)
    }
}
