use std::{
    mem::ManuallyDrop,
    ops::{Deref, DerefMut},
};

pub struct Defer<T, F: FnOnce(&mut T)> {
    value: ManuallyDrop<T>,
    f: Option<F>,
}

impl<T, F: FnOnce(&mut T)> Defer<T, F> {
    pub fn new(value: T, func: F) -> Self {
        Defer {
            value: ManuallyDrop::new(value),
            f: Some(func),
        }
    }

    #[allow(dead_code)]
    pub fn take(mut self) -> T {
        self.f = None;
        unsafe { ManuallyDrop::take(&mut self.value) }
    }
}

impl<T, F: FnOnce(&mut T)> Deref for Defer<T, F> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T, F: FnOnce(&mut T)> DerefMut for Defer<T, F> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.value
    }
}

impl<T, F> Drop for Defer<T, F>
where
    F: FnOnce(&mut T),
{
    fn drop(&mut self) {
        if let Some(x) = self.f.take() {
            (x)(&mut *self.value);
            unsafe { ManuallyDrop::drop(&mut self.value) }
        }
    }
}
