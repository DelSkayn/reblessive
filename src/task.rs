use std::{
    future::Future,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

#[derive(Debug)]
pub struct Task {
    ptr: NonNull<()>,
    dropper: unsafe fn(NonNull<()>),
    driver: unsafe fn(NonNull<()>, ctx: &mut Context<'_>) -> Poll<()>,
}

impl Task {
    pub fn wrap_future<F>(ptr: NonNull<F>) -> Self
    where
        F: Future<Output = ()>,
    {
        Task {
            ptr: ptr.cast(),
            dropper: Self::drop_impl::<F>,
            driver: Self::drive_impl::<F>,
        }
    }

    pub unsafe fn drive(&mut self, ctx: &mut Context<'_>) -> Poll<()> {
        (self.driver)(self.ptr, ctx)
    }

    pub unsafe fn drop(self) {
        (self.dropper)(self.ptr)
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
