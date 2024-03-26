use std::{alloc::Layout, ptr::NonNull, usize};

struct BlockHeader {
    previous: Option<NonNull<BlockHeader>>,
    head: *mut u8,
    last: *mut u8,
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
        let mut block = if let Some(b) = self.block {
            b
        } else {
            unsafe { self.grow_initial(layout) }
        };

        let size_required = Self::alloc_size(layout).unwrap();
        if unsafe {
            block.as_ref().head.offset_from(block.as_ptr().cast::<u8>()) as usize
                - std::mem::size_of::<BlockHeader>()
                < size_required
        } {
            // not enough space in existing block, allocate new block
            block = unsafe { self.grow_alloc(layout) };
        }

        let mut res = unsafe { block.as_ref().head.sub(layout.size()) };
        #[cfg(not(miri))]
        {
            res = ((res as usize) & !(layout.align() - 1)) as *mut u8;
        }
        #[cfg(miri)]
        unsafe {
            let offset = res as usize & layout.align() - 1;
            res = res.sub(offset);
        }

        unsafe { block.as_mut().head = block.as_ref().head.sub(size_required) };

        unsafe { NonNull::new_unchecked(res) }
    }

    /// Pop the top allocation from the allocator.
    /// # Safety
    /// Caller must ensure that the to be popped memory is no longer used and it was allocated with
    /// the same layout as given to this function.
    pub unsafe fn pop_dealloc(&mut self, layout: Layout) {
        let size = Self::alloc_size(layout).unwrap();
        // try to deallocate from the current block.
        let mut block = self.block.expect("invalid deallocation");
        let head = block.as_ref().head;
        if head != block.as_ref().last {
            block.as_mut().head = head.add(size);
            return;
        }

        let mut old_block = block.as_ref().previous.expect("invalid deallocation");
        let head = old_block.as_ref().head;
        assert_ne!(head, old_block.as_ref().last, "invalid deallocation");
        let head_add = head.add(size);

        if head_add == old_block.as_ref().last {
            block.as_mut().previous = old_block.as_ref().previous;
            Self::dealloc_old_block(old_block);
        } else {
            old_block.as_mut().head = head_add;
        }
    }

    // returns the amount of bytes required at most to allocate a value.
    // If the layout has an alignment bigger than that of the block header we allocate a space
    // larger then actually needed to ensure we can align the allocation pointer properly.
    const fn alloc_size(layout: Layout) -> Option<usize> {
        let pad_size = layout
            .align()
            .saturating_sub(std::mem::align_of::<BlockHeader>());
        layout.size().checked_add(pad_size)
    }

    #[cold]
    unsafe fn grow_alloc(&mut self, layout: Layout) -> NonNull<BlockHeader> {
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
        block
    }

    #[cold]
    unsafe fn grow_initial(&mut self, layout: Layout) -> NonNull<BlockHeader> {
        let required_size = Self::alloc_size(layout).unwrap() + std::mem::size_of::<BlockHeader>();

        // we failed to allocate so we need to allocate a new block.
        let new_alloc_size = required_size.next_power_of_two();
        assert_ne!(new_alloc_size, 0);

        let block = Self::alloc_new_block(new_alloc_size);
        self.block = Some(block);
        block
    }

    unsafe fn block_size(block: NonNull<BlockHeader>) -> usize {
        block.as_ref().last.offset_from(block.as_ptr().cast::<u8>()) as usize
    }

    unsafe fn alloc_new_block(size: usize) -> NonNull<BlockHeader> {
        debug_assert!(size.is_power_of_two());
        debug_assert!(size >= std::mem::size_of::<BlockHeader>());

        let layout = Layout::from_size_align(size, std::mem::align_of::<BlockHeader>()).unwrap();

        let ptr = NonNull::new(std::alloc::alloc(layout))
            .unwrap()
            .cast::<BlockHeader>();

        let head = ptr.cast::<u8>().as_ptr().add(size);

        ptr.as_ptr().write(BlockHeader {
            previous: None,
            head,
            last: head,
        });

        ptr
    }

    #[cold]
    unsafe fn dealloc_old_block(ptr: NonNull<BlockHeader>) {
        let size = ptr.as_ref().last.offset_from(ptr.as_ptr().cast::<u8>()) as usize;

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
