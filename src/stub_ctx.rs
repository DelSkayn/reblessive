use std::{
    ptr,
    task::{RawWaker, RawWakerVTable, Waker},
};

unsafe fn stub_clone(_: *const ()) -> RawWaker {
    // Casting to u8 is fine cause regardless of T the waker pointer will always be first.
    panic!("Called an non-reblessive async function withing a non-async reblessive context");
}

unsafe fn stub_wake(_: *const ()) {
    // Casting to u8 is fine cause regardless of T the waker pointer will always be first.
    panic!("Called an non-reblessive async function withing a non-async reblessive context");
}

unsafe fn stub_drop(_: *const ()) {}

static STUB_WAKER_V_TABLE: RawWakerVTable =
    RawWakerVTable::new(stub_clone, stub_wake, stub_wake, stub_drop);

pub fn get() -> Waker {
    unsafe { Waker::from_raw(RawWaker::new(ptr::null_mut(), &STUB_WAKER_V_TABLE)) }
}
