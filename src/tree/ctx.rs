use std::{
    ptr::NonNull,
    task::{RawWaker, RawWakerVTable, Waker},
};


use crate::Stack;

use super::schedular::Schedular;

unsafe fn stub_clone(ptr: *const ()) -> RawWaker {
    let ptr = NonNull::new_unchecked(ptr as *mut WakerCtx);
    std::mem::transmute::<Waker, RawWaker>(ptr.as_ref().waker.clone())
}

unsafe fn stub_wake(ptr: *const ()) {
    let ptr = NonNull::new_unchecked(ptr as *mut WakerCtx);
    ptr.as_ref().waker.wake_by_ref()
}

unsafe fn stub_drop(_: *const ()) {}

static TREE_WAKER_V_TABLE: RawWakerVTable =
    RawWakerVTable::new(stub_clone, stub_wake, stub_wake, stub_drop);

pub struct WakerCtx<'a> {
    pub waker: &'a Waker,
    pub stack: &'a Stack,
    pub fan: &'a Schedular,
}

impl<'a> WakerCtx<'a> {
    pub unsafe fn to_waker(&self) -> Waker {
        let raw = RawWaker::new(NonNull::from(self).as_ptr().cast(), &TREE_WAKER_V_TABLE);
        unsafe { Waker::from_raw(raw) }
    }

    pub fn ptr_from_waker(waker: &Waker) -> NonNull<Self> {
        let hack: &WakerHack = unsafe { std::mem::transmute(waker) };
        assert_eq!(hack.vtable,&TREE_WAKER_V_TABLE,"Found waker not created by reblessive stack, reblessive futures only work inside the reblessive executor");
        unsafe { NonNull::new_unchecked(hack.data as *mut Self) }
    }
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
    use super::{WakerHack, TREE_WAKER_V_TABLE};
    use std::task::{RawWaker, Waker};

    #[test]
    fn assert_hack_format() {
        // a more specific value to be more sure that values are actually equivalent.
        #[cfg(not(miri))]
        let data_ptr = 0x123456789abcdefusize as *mut ();
        // a pointer created by non null to not have to use a null ptr and avoiding provenance
        // warnings without using the unstable from_exposed_addr
        #[cfg(miri)]
        let data_ptr = std::ptr::NonNull::<()>::dangling().as_ptr();
        let raw_waker = RawWaker::new(data_ptr, &TREE_WAKER_V_TABLE);
        let waker = unsafe { Waker::from_raw(raw_waker) };
        let hack: WakerHack = unsafe { std::mem::transmute(waker) };
        assert_eq!(hack.data, data_ptr);
        assert_eq!(hack.vtable, &TREE_WAKER_V_TABLE);
    }
}
