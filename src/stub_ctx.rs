use std::{
    ptr::NonNull,
    task::{RawWaker, RawWakerVTable, Waker},
};

use crate::Stack;

unsafe fn stub_clone(ptr: *const ()) -> RawWaker {
    let ptr = NonNull::new_unchecked(ptr as *mut WakerCtx);
    if let Some(x) = ptr.as_ref().waker {
        std::mem::transmute(x.clone())
    } else {
        panic!("Called an non-reblessive async function withing a non-async reblessive context");
    }
}

unsafe fn stub_wake(ptr: *const ()) {
    let ptr = NonNull::new_unchecked(ptr as *mut WakerCtx);
    if let Some(x) = ptr.as_ref().waker {
        x.wake_by_ref()
    } else {
        panic!("Called an non-reblessive async function withing a non-async reblessive context");
    }
}

unsafe fn stub_drop(_: *const ()) {}

static STUB_WAKER_V_TABLE: RawWakerVTable =
    RawWakerVTable::new(stub_clone, stub_wake, stub_wake, stub_drop);

pub struct WakerCtx<'a> {
    pub stack: &'a Stack,
    pub waker: Option<&'a Waker>,
}

pub fn create(ptr: &WakerCtx) -> Waker {
    let raw = RawWaker::new(NonNull::from(ptr).as_ptr().cast(), &STUB_WAKER_V_TABLE);
    unsafe { Waker::from_raw(raw) }
}

pub fn get_stack(waker: &Waker) -> NonNull<WakerCtx> {
    let hack: &WakerHack = unsafe { std::mem::transmute(waker) };
    assert_eq!(hack.vtable,&STUB_WAKER_V_TABLE,"Found waker not created by reblessive stack, reblessive futures only work inside the reblessive executor");
    unsafe { NonNull::new_unchecked(hack.data as *mut WakerCtx) }
}

/// A struct with the same format as RawWaker.
/// Use as a hack until the getter functions for waker stablize.
struct WakerHack {
    data: *const (),
    vtable: &'static RawWakerVTable,
}

/// This is a static assertion validating the size of waker hack and waker to ensure they are the
/// same.
/// If this assertion fails there is something wrong with our hack definition.
#[allow(dead_code)]
const ASSERT_SIZE: [(); std::mem::size_of::<WakerHack>()] = [(); std::mem::size_of::<Waker>()];

#[cfg(test)]
mod test {
    use std::task::{RawWaker, Waker};

    use crate::stub_ctx::WakerHack;

    use super::STUB_WAKER_V_TABLE;

    #[test]
    fn assert_hack_format() {
        // a more specific value to be more sure that values are actually equivalent.
        #[cfg(not(miri))]
        let data_ptr = 0x123456789abcdefusize as *mut ();
        // a pointer created by non null to not have to use a null ptr and avoiding provenance
        // warnings without using the unstable from_exposed_addr
        #[cfg(miri)]
        let data_ptr = std::ptr::NonNull::<()>::dangling().as_ptr();
        let raw_waker = RawWaker::new(data_ptr, &STUB_WAKER_V_TABLE);
        let waker = unsafe { Waker::from_raw(raw_waker) };
        let hack: WakerHack = unsafe { std::mem::transmute(waker) };
        assert_eq!(hack.data, data_ptr);
        assert_eq!(hack.vtable, &STUB_WAKER_V_TABLE);
    }
}
