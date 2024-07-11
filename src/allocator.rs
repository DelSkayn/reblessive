use std::alloc::Layout;

use crate::ptr::Owned;

struct BlockHeader {
    previous: Option<Owned<BlockHeader>>,
    last: Owned<u8>,
}

/// A stack allocator, an allocator which is only able to free the most recent allocated value.
///
/// Allocates increasingly larger and larger chunks of memory, freeing previous ones once they are
/// empty, only keeping the most recent around.
pub struct StackAllocator {
    block: Option<Owned<BlockHeader>>,
    top: Option<Owned<u8>>,
}

impl StackAllocator {
    pub const MINIMUM_ALIGN: usize = std::mem::align_of::<Option<Owned<u8>>>();
    pub const BACK_POINTER_SIZE: usize = std::mem::size_of::<Option<Owned<u8>>>();

    pub fn new() -> Self {
        StackAllocator {
            block: None,
            top: None,
        }
    }

    pub fn top(&self) -> Option<Owned<u8>> {
        self.top.map(|x| unsafe { x.add(Self::BACK_POINTER_SIZE) })
    }

    // returns the amount of bytes required at most to allocate a value.
    // If the layout has an alignment bigger than that of the block header we allocate a space
    // larger then actually needed to ensure we can align the allocation pointer properly.
    const fn alloc_size(layout: Layout) -> Option<usize> {
        let pad_size = layout.align().saturating_sub(Self::MINIMUM_ALIGN);
        layout
            .size()
            .checked_add(pad_size + Self::BACK_POINTER_SIZE)
    }

    fn within_block(block: Owned<BlockHeader>, addr: usize) -> bool {
        let size = unsafe { Self::block_size(block) };
        let block_addr = block.addr().get();
        (block_addr..=(block_addr + size)).contains(&addr)
    }

    /// Creates an allocator with reserve capacity for atleast cap bytes of space not taking into
    /// account alignment.
    pub fn with_capacity(cap: usize) -> Self {
        let layout = Layout::new::<BlockHeader>();
        let block_size =
            layout.size() + cap.next_power_of_two() + Layout::new::<BlockHeader>().size();

        StackAllocator {
            block: Some(unsafe { Self::alloc_new_block(block_size) }),
            top: None,
        }
    }

    pub fn alloc<T>(&mut self, t: T) -> Owned<T> {
        self.alloc_with(|| t)
    }

    #[inline(always)]
    pub fn alloc_with<T, F: FnOnce() -> T>(&mut self, f: F) -> Owned<T> {
        #[inline(always)]
        unsafe fn inner_writer<T, F>(ptr: *mut T, f: F)
        where
            F: FnOnce() -> T,
        {
            std::ptr::write(ptr, f())
        }

        let layout = Layout::new::<T>();

        let ptr = self.alloc_layout(layout).cast::<T>();
        unsafe { inner_writer(ptr.as_ptr(), f) }
        ptr
    }

    /// Push a new allocation to the top of the allocator.
    pub fn alloc_layout(&mut self, layout: Layout) -> Owned<u8> {
        let block = if let Some(b) = self.block {
            b
        } else {
            unsafe { self.grow_initial(layout) }
        };

        let mut top_ptr = self
            .top
            .map(|x| {
                let x = x.cast::<u8>();
                if !Self::within_block(block, x.addr().get()) {
                    // only allocate on the last block.
                    unsafe { block.as_ref().last }
                } else {
                    x
                }
            })
            .unwrap_or_else(|| unsafe { block.as_ref().last });

        let alloc_size = Self::alloc_size(layout).expect("Type layour exceeded limits");

        let remaining = top_ptr
            .addr()
            .get()
            .saturating_sub(alloc_size)
            .saturating_sub(block.addr().get());

        if remaining < std::mem::size_of::<BlockHeader>() {
            top_ptr = unsafe { self.grow_alloc_new_top(layout) };
        }

        let align = layout.align().max(Self::MINIMUM_ALIGN);
        let size = layout.size();

        let res = unsafe { top_ptr.sub(size) };
        let res = unsafe { res.map_addr_unchecked(|addr| addr & !(align - 1)) };
        unsafe {
            let new_top = res.sub(Self::BACK_POINTER_SIZE);
            new_top.cast().write(self.top);
            self.top = Some(new_top);
        }

        res
    }

    /// Pop the top allocation from the allocator.
    /// # Safety
    /// Caller must ensure that the to be popped memory is no longer used and it was allocated with
    /// the same layout as given to this function.
    pub unsafe fn pop_dealloc(&mut self) {
        let top = self.top.expect("invalid deallocation");
        // if there is a top, there must be a block.
        let mut block = self.block.unwrap();
        self.top = top.cast::<Option<Owned<u8>>>().read();

        if Self::within_block(block, top.addr().get()) {
            return;
        }

        // the current top was not allocated on the head block,
        // so it must be on the old one.
        let old_block = block.as_ref().previous.expect("invalid deallocation");
        if self
            .top
            .map(|x| !Self::within_block(old_block, x.addr().get()))
            .unwrap_or(true)
        {
            // top was either None, meaning nothing is allocated, or the new head is not in the
            // current block, meaning that this block must now be empty, so can be deallcated.
            block.as_mut().previous = old_block.as_ref().previous;
            Self::dealloc_old_block(old_block);
        }
    }

    #[cold]
    unsafe fn grow_alloc_new_top(&mut self, layout: Layout) -> Owned<u8> {
        let required_size = Self::alloc_size(layout).unwrap();
        let old_block = self.block.take().unwrap();

        // we failed to allocate so we need to allocate a new block.
        let new_alloc_size = unsafe {
            Self::block_size(old_block)
                .checked_add(required_size)
                .unwrap()
                .next_power_of_two()
        };
        let mut block = Self::alloc_new_block(new_alloc_size);

        block.as_mut().previous = Some(old_block);
        self.block = Some(block);

        block.as_ref().last
    }

    #[cold]
    unsafe fn grow_initial(&mut self, layout: Layout) -> Owned<BlockHeader> {
        let required_size = Self::alloc_size(layout).unwrap() + std::mem::size_of::<BlockHeader>();

        // we failed to allocate so we need to allocate a new block.
        let new_alloc_size = required_size.next_power_of_two();
        assert_ne!(new_alloc_size, 0);

        let block = Self::alloc_new_block(new_alloc_size);
        self.block = Some(block);
        block
    }

    unsafe fn block_size(block: Owned<BlockHeader>) -> usize {
        block.as_ref().last.offset_from(block.cast::<u8>()) as usize
    }

    unsafe fn alloc_new_block(size: usize) -> Owned<BlockHeader> {
        debug_assert!(size.is_power_of_two());
        debug_assert!(size >= std::mem::size_of::<BlockHeader>());
        assert!(
            size < isize::MAX as usize,
            "Exceeded maximum allocation size"
        );

        let layout = Layout::from_size_align(size, std::mem::align_of::<BlockHeader>()).unwrap();

        let ptr = Owned::from_ptr(std::alloc::alloc(layout))
            .unwrap()
            .cast::<BlockHeader>();

        let head = ptr.cast::<u8>().add(size);

        ptr.as_ptr().write(BlockHeader {
            previous: None,
            last: head,
        });

        ptr
    }

    #[cold]
    unsafe fn dealloc_old_block(ptr: Owned<BlockHeader>) {
        let size = ptr.as_ref().last.offset_from(ptr.cast::<u8>()) as usize;

        let layout = Layout::from_size_align(size, std::mem::align_of::<BlockHeader>()).unwrap();

        std::alloc::dealloc(ptr.as_ptr().cast(), layout)
    }
}

impl Drop for StackAllocator {
    fn drop(&mut self) {
        let mut cur = self.block;
        while let Some(b) = cur {
            unsafe {
                cur = b.as_ref().previous;
                Self::dealloc_old_block(b);
            }
        }
    }
}

#[cfg(test)]
mod test {
    use crate::ptr::{map_ptr, Owned};

    use super::StackAllocator;

    pub struct Ballast<V, const SIZE: usize> {
        v: V,
        _other: [usize; SIZE],
    }

    impl<V: PartialEq, const SIZE: usize> PartialEq for Ballast<V, SIZE> {
        fn eq(&self, other: &Self) -> bool {
            self.v.eq(&other.v)
        }
    }

    #[test]
    fn test_allocation() {
        unsafe {
            let mut alloc = StackAllocator::new();
            let mut allocations = Vec::<Owned<Ballast<usize, 64>>>::new();

            let amount: usize = if cfg!(miri) { 10 } else { 1000 };

            for i in 0..amount {
                let b = Ballast {
                    v: i,
                    _other: [0usize; 64],
                };
                let alloc = alloc.alloc(b);
                assert!(!allocations.contains(&alloc));
                allocations.push(alloc);
            }

            for (i, v) in allocations.iter().enumerate() {
                assert_eq!(i, v.map_ptr(map_ptr!(Ballast<_,_>,v)).read())
            }

            for _ in 0..(amount / 2) {
                assert_eq!(allocations.last().map(|x| x.cast()), alloc.top());
                allocations.pop();
                alloc.pop_dealloc();
            }

            let mut allocations_2 = Vec::new();

            for i in 0..amount {
                let alloc = alloc.alloc(i as u128);
                assert!(!allocations_2.contains(&alloc));
                allocations_2.push(alloc);
            }

            for (i, v) in allocations_2.iter().enumerate() {
                assert_eq!(i as u128, v.as_ptr().read())
            }
        }
    }
}
