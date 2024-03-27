use std::{
    alloc::Layout,
    future::Future,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

#[cfg(feature = "tree")]
use crate::tree;

/// A constant table generated for each type of tasks that is spawned.
#[derive(Debug, Clone)]
pub(crate) struct VTable {
    /// Funtion to drop the task in place.
    pub(crate) dropper: unsafe fn(NonNull<u8>),
    /// Funtion to drive the task forward.
    pub(crate) driver: unsafe fn(NonNull<u8>, ctx: &mut Context<'_>) -> Poll<()>,
    /// The layout of the future.
    pub(crate) layout: Layout,
    #[cfg(feature = "tree")]
    /// The layout of the future when inside the schedular::Task  struct.
    pub(crate) tree: tree::SchedularVTable,
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
                layout: Layout::new::<F>(),
                #[cfg(feature = "tree")]
                tree: tree::SchedularVTable::get::<F>(),
            };
        }

        &<F as HasVTable>::V_TABLE
    }

    unsafe fn drop_impl<F>(ptr: NonNull<u8>)
    where
        F: Future<Output = ()>,
    {
        std::ptr::drop_in_place(ptr.cast::<F>().as_ptr())
    }

    unsafe fn drive_impl<F>(ptr: NonNull<u8>, ctx: &mut Context<'_>) -> Poll<()>
    where
        F: Future<Output = ()>,
    {
        Pin::new_unchecked(ptr.cast::<F>().as_mut()).poll(ctx)
    }
}
