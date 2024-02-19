use std::{
    ptr,
    task::{RawWaker, RawWakerVTable, Waker},
};

unsafe fn stub_clone(_: *const ()) -> RawWaker {
    panic!("CLONE, Called an non-reblessive async function withing a non-async reblessive context");
}

unsafe fn stub_wake(_: *const ()) {
    panic!("Called an non-reblessive async function withing a non-async reblessive context");
}
unsafe fn stub_drop(_: *const ()) {}

static STUB_WAKER_V_TABLE: RawWakerVTable =
    RawWakerVTable::new(stub_clone, stub_wake, stub_wake, stub_drop);

pub fn get() -> Waker {
    let raw = RawWaker::new(ptr::null(), &STUB_WAKER_V_TABLE);
    unsafe { Waker::from_raw(raw) }
}
