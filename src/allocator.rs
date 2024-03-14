use std::{alloc::Layout, ptr::NonNull, usize};

struct BlockHeader {
    previous: Option<NonNull<BlockHeader>>,
    used: usize,
    size: usize,
}

/// A stack allocator, an allocator which is only able to free the most recent allocated value.
///
/// Allocates increasingly larger and larger chunks of memory, freeing previous ones once they are
/// empty, only keeping the most recent around.
pub struct StackAllocator {
    block: Option<NonNull<BlockHeader>>,
}

impl StackAllocator {
    pub fn new() -> Self {
        StackAllocator { block: None }
    }

    /// Creates an allocator with reserve capacity for atleast cap bytes of space not taking into
    /// account alignment.
    pub fn with_capacity(cap: usize) -> Self {
        let layout = Layout::new::<BlockHeader>();
        let block_size = layout.size() + cap.next_power_of_two();

        StackAllocator {
            block: Some(unsafe { Self::alloc_new_block(block_size) }),
        }
    }

    /// Push a new allocation to the top of the allocator.
    pub fn push_alloc(&mut self, layout: Layout) -> NonNull<u8> {
        let block = if let Some(b) = self.block {
            b
        } else {
            // No block, so first allocation, allocate a block which can contain the current to be
            // allocated layout.
            let block = unsafe { Self::alloc_new_block_for_layout(layout) };
            self.block = Some(block);
            block
        };

        // try to allocate within the current block.
        if let Some(res) = unsafe { Self::alloc_within_block(block, layout) } {
            return unsafe { NonNull::new_unchecked(Self::align_up(res.as_ptr(), layout)) };
        };

        // we failed to allocate so we need to allocate a new block.
        let size = unsafe {
            block
                .as_ref()
                .size
                .checked_add(Self::alloc_size(layout))
                .unwrap()
                .next_power_of_two()
        };
        assert_ne!(size, 0);

        let mut new_block = unsafe { Self::alloc_new_block(size) };
        unsafe { new_block.as_mut().previous = Some(block) };
        self.block = Some(new_block);
        let res = unsafe { Self::alloc_within_block(new_block, layout).unwrap() };
        unsafe { NonNull::new_unchecked(Self::align_up(res.as_ptr(), layout)) }
    }

    /// Pop the top allocation from the allocator.
    /// # Safety
    /// Caller must ensure that the to be popped memory is no longer used and it was allocated with
    /// the same layout as given to this function.
    pub unsafe fn pop_dealloc(&mut self, layout: Layout) {
        let size = Self::alloc_size(layout);
        // try to deallocate from the current block.
        let mut block = self.block.expect("invalid deallocation");
        if block.as_ref().used > 0 {
            assert!(block.as_ref().used >= size, "invalid deallocation");
            block.as_mut().used -= size;
            return;
        }
        // current block was empty so the allocation must be in the previous block.
        let mut old_block = block.as_ref().previous.expect("invalid deallocation");
        assert!(old_block.as_ref().used >= size, "invalid deallocation");
        old_block.as_mut().used -= size;
        if old_block.as_ref().used == 0 {
            // previous block is now empty so deallocate the block.
            // This also ensures that we always only have to look within the current or previous
            // block when deallocating.
            block.as_mut().previous = old_block.as_ref().previous;
            Self::dealloc_old_block(old_block);
        }
    }

    // returns the amount of bytes required at most to allocate a value.
    // If the layout has an alignment bigger than that of the block header we allocate a space
    // larger then actually needed to ensure we can align the allocation pointer properly.
    fn alloc_size(layout: Layout) -> usize {
        let pad_size = layout
            .align()
            .saturating_sub(std::mem::align_of::<BlockHeader>());
        layout.size().checked_add(pad_size).unwrap()
    }

    #[cold]
    unsafe fn alloc_new_block_for_layout(layout: Layout) -> NonNull<BlockHeader> {
        let size = Self::alloc_size(layout).next_power_of_two();
        assert_ne!(size, 0);
        Self::alloc_new_block(size)
    }

    #[cold]
    unsafe fn alloc_new_block(size: usize) -> NonNull<BlockHeader> {
        debug_assert!(size.is_power_of_two());

        let header_layout = Layout::new::<BlockHeader>();
        let space_layout = Layout::from_size_align(size, header_layout.align()).unwrap();
        let (block_layout, offset) = Layout::new::<BlockHeader>().extend(space_layout).unwrap();

        assert_eq!(std::mem::size_of::<BlockHeader>(), offset);

        let ptr = NonNull::new(std::alloc::alloc(block_layout))
            .unwrap()
            .cast::<BlockHeader>();

        ptr.as_ptr().write(BlockHeader {
            previous: None,
            used: 0,
            size,
        });

        ptr
    }

    #[cold]
    unsafe fn dealloc_old_block(ptr: NonNull<BlockHeader>) {
        let size = ptr.as_ref().size;
        let header_layout = Layout::new::<BlockHeader>();
        let space_layout = Layout::from_size_align(size, header_layout.align()).unwrap();
        let (block_layout, _) = Layout::new::<BlockHeader>().extend(space_layout).unwrap();

        std::alloc::dealloc(ptr.as_ptr().cast(), block_layout)
    }

    unsafe fn alloc_within_block(
        mut block: NonNull<BlockHeader>,
        layout: Layout,
    ) -> Option<NonNull<u8>> {
        let size = Self::alloc_size(layout);
        let used = block.as_ref().used;
        if block.as_ref().size - used < size {
            return None;
        }

        let res = block.as_ptr().add(1).cast::<u8>().add(used);
        block.as_mut().used += size;
        Some(NonNull::new_unchecked(res))
    }

    unsafe fn align_up(ptr: *mut u8, layout: Layout) -> *mut u8 {
        let offset = ptr.align_offset(layout.align());
        assert_ne!(offset, usize::MAX);
        ptr.add(offset)
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
    use std::alloc::Layout;

    use super::StackAllocator;

    #[test]
    fn test_allocation() {
        unsafe {
            let mut alloc = StackAllocator::new();
            let mut allocations = Vec::new();

            #[cfg(not(miri))]
            let amount = 1000;
            #[cfg(miri)]
            let amount = 10;

            for i in 0..amount {
                let alloc = alloc.push_alloc(Layout::new::<usize>()).cast::<usize>();
                alloc.as_ptr().write(i);
                assert!(!allocations.contains(&alloc));
                allocations.push(alloc);
            }

            for (i, v) in allocations.iter().enumerate() {
                assert_eq!(i, v.as_ptr().read())
            }

            for _ in 0..(amount / 2) {
                allocations.pop();
                alloc.pop_dealloc(Layout::new::<usize>());
            }

            let mut allocations_2 = Vec::new();

            for i in 0..amount {
                let alloc = alloc.push_alloc(Layout::new::<u128>()).cast::<u128>();
                alloc.as_ptr().write(i as u128);
                assert!(!allocations_2.contains(&alloc));
                allocations_2.push(alloc);
            }

            for (i, v) in allocations_2.iter().enumerate() {
                assert_eq!(i as u128, v.as_ptr().read())
            }
        }
    }
}
