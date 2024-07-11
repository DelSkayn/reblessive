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
        Self {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }
}

impl<T> Owned<T> {
    pub fn into_ref<'a>(self) -> Ref<'a, T> {
        Ref {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }

    pub fn into_mut<'a>(self) -> Mut<'a, T> {
        Mut {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }

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

pub struct Ref<'a, T> {
    ptr: NonNull<T>,
    _marker: PhantomData<&'a T>,
}

impl<T> Copy for Ref<'_, T> {}
impl<T> Clone for Ref<'_, T> {
    fn clone(&self) -> Self {
        Self {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }
}

impl<'a, T> Ref<'a, T> {
    pub fn into_owned(self) -> Owned<T> {
        Owned {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }

    pub unsafe fn read(&self) -> T {
        self.ptr.as_ptr().read()
    }
}

impl<'a, T> From<&'a T> for Ref<'a, T> {
    fn from(value: &'a T) -> Self {
        Ref {
            ptr: NonNull::from(value),
            _marker: PhantomData,
        }
    }
}

impl<'a, T> From<&'a mut T> for Ref<'a, T> {
    fn from(value: &'a mut T) -> Self {
        Ref {
            ptr: NonNull::from(value),
            _marker: PhantomData,
        }
    }
}

impl<'a, T> From<Mut<'a, T>> for Ref<'a, T> {
    fn from(value: Mut<'a, T>) -> Self {
        Ref {
            ptr: value.ptr,
            _marker: PhantomData,
        }
    }
}

pub struct Mut<'a, T> {
    ptr: NonNull<T>,
    _marker: PhantomData<&'a mut T>,
}

impl<T> Clone for Mut<'_, T> {
    fn clone(&self) -> Self {
        Self {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }
}

impl<'a, T> Mut<'a, T> {
    pub unsafe fn write(&self, t: T) {
        self.ptr.as_ptr().write(t)
    }

    pub unsafe fn read(&self) -> T {
        self.ptr.as_ptr().read()
    }

    pub unsafe fn replace(&self, v: T) -> T {
        std::ptr::replace(self.as_ptr(), v)
    }

    pub fn into_owned(self) -> Owned<T> {
        Owned {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }

    pub fn into_ref(self) -> Ref<'a, T> {
        Ref {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }

    pub fn reborrow<'b>(&mut self) -> Mut<'b, T> {
        Mut {
            ptr: self.ptr,
            _marker: PhantomData,
        }
    }
}

impl<'a, T> From<&'a mut T> for Mut<'a, T> {
    fn from(value: &'a mut T) -> Self {
        Mut {
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
                self.ptr.partial_cmp(&other.ptr)
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

            pub fn from_nonnull(ptr: NonNull<$gen>) -> Self {
                Self {
                    ptr,
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

            pub fn into_ptr(self) -> *mut $gen {
                self.ptr.as_ptr()
            }

            pub fn as_ptr(&self) -> *mut $gen {
                self.ptr.as_ptr()
            }

            pub fn into_nonnull(self) -> NonNull<$gen> {
                self.ptr
            }

            pub fn as_nonnull(&self) -> NonNull<$gen> {
                self.ptr
            }

            pub unsafe fn as_ref(&self) -> &$($lt)? $gen {
                self.ptr.as_ref()
            }

            pub unsafe fn as_mut(&mut self) -> &$($lt)? mut $gen {
                self.ptr.as_mut()
            }

            pub unsafe fn add(self, offset: usize) -> Self{
                self.map_addr(|x| NonZeroUsize::new_unchecked(x.get() + offset))
            }

            pub unsafe fn sub(self, offset: usize) -> Self{
                self.map_addr(|x| NonZeroUsize::new_unchecked(x.get() - offset))
            }

            pub unsafe fn offset(self, offset: isize) -> Self{
                self.map_addr(|x| NonZeroUsize::new_unchecked((x.get() as isize + offset) as usize))
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

            pub fn expose_provenance(self) -> NonZeroUsize{
                #[cfg(feature = "nightly")]
                {
                    unsafe{ NonZeroUsize::new_unchecked(self.ptr.as_ptr().expose_provenance()) }
                }
                #[cfg(not(feature = "nightly"))]
                {
                    unsafe{ NonZeroUsize::new_unchecked(self.ptr.as_ptr() as usize) }
                }
            }

            pub fn from_exposed_addr(addr: NonZeroUsize) -> Self{
                #[cfg(feature = "nightly")]
                {
                    Self{
                        ptr: unsafe{ NonNull::new_unchecked(std::ptr::with_exposed_provenance_mut(addr.get())) },
                        _marker: PhantomData,
                    }
                }
                #[cfg(not(feature = "nightly"))]
                {

                    Self{
                        ptr: unsafe{ NonNull::new_unchecked(addr.get() as *mut $gen) },
                        _marker: PhantomData,
                    }
                }
            }

            pub fn with_addr(self, addr: NonZeroUsize) -> Self{
                #[cfg(feature = "nightly")]
                {
                    unsafe{ Self::from_ptr_unchecked(self.ptr.as_ptr().with_addr(addr.get())) }
                }
                #[cfg(not(feature = "nightly"))]
                {
                    unsafe{ Self::from_ptr_unchecked(addr.get() as *mut $gen) }
                }
            }

            pub fn map_addr<F>(self, f: F) -> Self
            where F: FnOnce(NonZeroUsize) -> NonZeroUsize
            {
                #[cfg(feature = "nightly")]
                {
                    unsafe{
                        Self::from_ptr_unchecked(self.ptr.as_ptr().map_addr(|x| {
                        f(NonZeroUsize::new_unchecked(x)).get()
                    }))
                    }
                }
                #[cfg(not(feature = "nightly"))]
                {
                    Self::from_exposed_addr(f(self.expose_provenance()))
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
                    Self::from_exposed_addr(NonZeroUsize::new_unchecked(f(self.expose_provenance().get())))
                }
            }

        }
    };
}

impl_base_methods!(Ref<'a, T>);
impl_base_methods!(Mut<'a, T>);
impl_base_methods!(Owned<T>);
