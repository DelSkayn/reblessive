use std::{fmt, marker::PhantomData, num::NonZeroUsize, ptr::NonNull};

#[macro_use]
mod mac {
    #[doc(hidden)]
    #[macro_export]
    macro_rules! __map_ptr {
        ($ty:ty $(,$field:ident)*) => {
            |x| {
                let ptr = x;
                $(let ptr = std::ptr::addr_of_mut!((*ptr).$field);)*
                ptr
            }
        };
    }
}
pub(crate) use crate::__map_ptr as map_ptr;

pub struct Owned<T> {
    ptr: NonNull<T>,
    _marker: PhantomData<T>,
}

impl<T> Copy for Owned<T> {}
impl<T> Clone for Owned<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Owned<T> {
    pub unsafe fn write(&self, v: T) {
        self.ptr.as_ptr().write(v);
    }

    pub unsafe fn read(&self) -> T {
        self.ptr.as_ptr().read()
    }

    pub unsafe fn replace(&self, v: T) -> T {
        std::ptr::replace(self.as_ptr(), v)
    }
}

impl<T> From<&T> for Owned<T> {
    fn from(t: &T) -> Self {
        Owned {
            ptr: NonNull::from(t),
            _marker: PhantomData,
        }
    }
}

impl<T> From<&mut T> for Owned<T> {
    fn from(value: &mut T) -> Self {
        Owned {
            ptr: NonNull::from(value),
            _marker: PhantomData,
        }
    }
}

macro_rules! impl_base_methods {
    ($ty:ident<$($lt:lifetime,)?$gen:ident>) => {


        impl<$($lt,)?$gen>  fmt::Debug for $ty<$($lt,)?$gen>{
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct("Owned").field("ptr", &self.ptr).finish()
            }
        }

        impl<$($lt,)?$gen>  PartialEq for $ty<$($lt,)?$gen>{
            fn eq(&self, other: &Self) -> bool {
                self.ptr == other.ptr
            }
        }

        impl<$($lt,)?$gen>  Eq for $ty<$($lt,)?$gen>{
        }


        impl<$($lt,)?$gen> PartialOrd for $ty<$($lt,)?$gen> {
            fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
                Some(self.ptr.cmp(&other.ptr))
            }
        }

        impl<$($lt,)?$gen> Ord for $ty<$($lt,)?$gen> {
            fn cmp(&self, other: &Self) -> std::cmp::Ordering {
                self.ptr.cmp(&other.ptr)
            }
        }

        impl<$($lt,)?$gen> $ty<$($lt,)?$gen> {
            pub fn dangling() -> Self{
                Self {
                    ptr: NonNull::dangling(),
                    _marker: PhantomData,
                }
            }


            pub fn from_ptr(ptr: *mut $gen) -> Option<Self> {
                NonNull::new(ptr).map(|x| Self {
                    ptr: x,
                    _marker: PhantomData,
                })
            }

            pub unsafe fn from_ptr_unchecked(ptr: *mut $gen) -> Self {
                Self {
                    ptr: NonNull::new_unchecked(ptr),
                    _marker: PhantomData,
                }
            }

            pub fn as_ptr(&self) -> *mut $gen {
                self.ptr.as_ptr()
            }

            pub unsafe fn as_ref<'a>(self) -> &'a $($lt)? $gen {
                self.ptr.as_ref()
            }

            pub unsafe fn as_mut<'a>(mut self) -> &'a $($lt)? mut $gen {
                self.ptr.as_mut()
            }

            pub unsafe fn add(self, offset: usize) -> Self{
                Self{
                    ptr: NonNull::new_unchecked(self.ptr.as_ptr().add(offset)),
                    _marker: PhantomData
                }
            }

            pub unsafe fn sub(self, offset: usize) -> Self{
                Self{
                    ptr: NonNull::new_unchecked(self.ptr.as_ptr().sub(offset)),
                    _marker: PhantomData
                }
            }

            pub unsafe fn offset_from(self, other: $ty<$gen>) -> isize{
                self.ptr.as_ptr().offset_from(other.ptr.as_ptr())
            }

            pub unsafe fn map_ptr<F,R>(self, f: F) -> $ty<$($lt,)?R>
                where F: FnOnce(*mut T) -> *mut R
            {
                $ty::from_ptr_unchecked(f(self.as_ptr()))
            }

            pub fn cast<R>(self) -> $ty<$($lt,)?R>{
                $ty{
                    ptr: self.ptr.cast(),
                    _marker: PhantomData,
                }
            }

            pub fn addr(self) -> NonZeroUsize{
                #[cfg(feature = "nightly")]
                {
                    unsafe{ NonZeroUsize::new_unchecked(self.ptr.as_ptr().addr()) }
                }
                #[cfg(not(feature = "nightly"))]
                {
                    unsafe{ NonZeroUsize::new_unchecked(self.ptr.as_ptr() as usize) }
                }
            }

            pub unsafe fn map_addr_unchecked<F>(self, f: F) -> Self
            where F: FnOnce(usize) -> usize
            {
                #[cfg(feature = "nightly")]
                {
                    unsafe{
                        Self::from_ptr_unchecked(self.ptr.as_ptr().map_addr(f))
                    }
                }
                #[cfg(not(feature = "nightly"))]
                {
                    unsafe{
                        let ptr = f(self.addr().get()) as *mut $gen;
                        Self{
                            ptr: NonNull::new_unchecked(ptr),
                            _marker: PhantomData
                        }
                    }
                }
            }

        }
    };
}

impl_base_methods!(Owned<T>);
