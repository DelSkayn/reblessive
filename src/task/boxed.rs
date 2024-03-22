use std::{
    alloc::Layout,
    pin::Pin,
    ptr::NonNull,
    task::{Context, Poll},
};

use futures_util::Future;

use super::TaskVTable;

pub struct BoxedTask {
    v_table: &'static TaskVTable,
    ptr: NonNull<u8>,
}

impl BoxedTask {
    /// Create a new boxed task.
    ///
    /// # Safety
    /// This function erases any possible lifetime associated with future.
    /// Caller must ensure that the future handed to this function outlives the object returned by
    /// the function.
    pub unsafe fn new<F>(f: F) -> Self
    where
        F: Future<Output = ()>,
    {
        let v_table = TaskVTable::get::<F>();
        let ptr = unsafe {
            NonNull::new(std::alloc::alloc(Layout::new::<F>()))
                .expect("Allocation failed")
                .cast::<F>()
        };
        unsafe {
            ptr.as_ptr().write(f);
        }
        BoxedTask {
            v_table,
            ptr: ptr.cast(),
        }
    }

    /// Poll the future in this task.
    pub fn poll(&mut self, cx: &mut Context) -> Poll<()> {
        unsafe { (self.v_table.driver)(self.ptr, cx) }
    }
}

impl Unpin for BoxedTask {}
impl Future for BoxedTask {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.get_mut().poll(cx)
    }
}

impl Drop for BoxedTask {
    fn drop(&mut self) {
        unsafe {
            (self.v_table.dropper)(self.ptr);
            std::alloc::dealloc(self.ptr.as_ptr(), self.v_table.layout)
        }
    }
}
