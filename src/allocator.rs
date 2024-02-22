use std::{
    alloc::Layout,
    ptr::{addr_of_mut, NonNull},
    usize,
};

struct BlockHeader {
    previous: Option<NonNull<BlockHeader>>,
    top: NonNull<u8>,
    size: usize,
}

#[repr(C)]
struct Allocation<T> {
    prev_size: usize,
    value: T,
}

/// A stack allocator, an allocator which is only able to free the most recent allocated value.
///
/// Allocates increasingly larger and larger chunks of memory, freeing previous ones once they are
/// empty, only keeping the most recent around.
pub struct StackAllocator {
    block: Option<NonNull<BlockHeader>>,
    last_alloc_size: usize,
    allocations: usize,
}

impl StackAllocator {
    pub fn new() -> Self {
        StackAllocator {
            block: None,
            last_alloc_size: 0,
            allocations: 0,
        }
    }

    pub fn with_capacity(cap: usize) -> Self {
        let layout = Layout::new::<BlockHeader>();
        let block_size = layout.size() + cap.next_power_of_two();

        StackAllocator {
            block: Some(unsafe { Self::alloc_new_block(block_size, None) }),
            last_alloc_size: 0,
            allocations: 0,
        }
    }

    unsafe fn block_empty(block: NonNull<BlockHeader>) -> bool {
        block.as_ref().top.as_ptr()
            == block
                .cast::<u8>()
                .as_ptr()
                .add(std::mem::size_of::<BlockHeader>())
    }

    pub unsafe fn head(&self) -> Option<NonNull<u8>> {
        let block = self.block?;
        if Self::block_empty(block) {
            let block = block.as_ref().previous?;
            if Self::block_empty(block) {
                return None;
            }
            Some(NonNull::new_unchecked(
                block.cast::<u8>().as_ptr().sub(self.last_alloc_size),
            ))
        } else {
            Some(NonNull::new_unchecked(
                block.cast::<u8>().as_ptr().sub(self.last_alloc_size),
            ))
        }
    }

    unsafe fn alloc_new_block(
        size: usize,
        previous: Option<NonNull<BlockHeader>>,
    ) -> NonNull<BlockHeader> {
        debug_assert!(size.is_power_of_two());

        let alloc_layout = Layout::from_size_align(size, 8).unwrap();
        let allocation = NonNull::new(std::alloc::alloc(alloc_layout))
            .unwrap()
            .cast::<BlockHeader>();

        let top = NonNull::new_unchecked(
            allocation
                .cast::<u8>()
                .as_ptr()
                .add(std::mem::size_of::<BlockHeader>()),
        );

        allocation
            .as_ptr()
            .cast::<BlockHeader>()
            .write(BlockHeader {
                previous,
                top,
                size,
            });

        allocation
    }

    pub fn alloc<T>(&mut self) -> NonNull<T> {
        // Get the current top block, if there is none create one.
        let mut block = if let Some(x) = self.block {
            x
        } else {
            // Create based on the size of the current future.
            let (layout, _) = Layout::new::<BlockHeader>()
                .extend(Layout::new::<Allocation<T>>())
                .unwrap();
            let layout = layout.pad_to_align();
            let size = layout.size() + layout.align().saturating_sub(8);
            let block_size = size.next_power_of_two();

            let alloc = unsafe { Self::alloc_new_block(block_size, self.block) };
            self.block = Some(alloc);
            alloc
        };

        unsafe {
            // we have a block try to allocate in it.
            let layout = Layout::new::<Allocation<T>>();
            let top = block.as_ref().top;
            let extra_offset = top.as_ptr().align_offset(layout.align());
            // align_offset is allowed to return usize::MAX when it can't allign a pointer.
            assert_ne!(extra_offset, usize::MAX, "Couldn't align allocation");

            let required_size = layout.size() + extra_offset;

            let size = block.as_ref().size;
            debug_assert!(size.is_power_of_two());
            let used_size = top.as_ptr().offset_from(block.cast::<u8>().as_ptr()) as usize;

            if size - used_size >= required_size {
                // we have enough space in the current block.

                let tgt = top.as_ptr().add(extra_offset).cast::<Allocation<T>>();

                // write the size of the previous allocation
                tgt.cast::<usize>()
                    .write(self.last_alloc_size + extra_offset);

                block.as_mut().top = NonNull::new_unchecked(top.as_ptr().add(required_size));
                self.last_alloc_size = required_size;
                self.allocations += 1;

                NonNull::new_unchecked(addr_of_mut!((*tgt).value))
            } else {
                // we don't have enough space in the current block so we need to allocate a new
                // one.

                // Make the new block twice the size of the old block.
                let (layout, _) = Layout::new::<BlockHeader>()
                    .extend(Layout::new::<Allocation<T>>())
                    .unwrap();
                let layout = layout.pad_to_align();
                // make sure the new block can fit the new allocation.
                let new_size = (size << 1).max(layout.size().next_power_of_two());

                let mut block = Self::alloc_new_block(new_size, self.block);
                self.block = Some(block);

                let top = block.as_ref().top;

                let extra_offset = top.as_ptr().align_offset(layout.align());
                let required_size = Layout::new::<Allocation<T>>().size() + extra_offset;

                assert_ne!(extra_offset, usize::MAX, "Couldn't align allocation");
                assert!(
                    top.as_ptr().offset_from(block.as_ptr().cast()) as usize + required_size
                        < new_size
                );

                let tgt = top.as_ptr().add(extra_offset).cast::<Allocation<T>>();
                tgt.cast::<usize>().write(self.last_alloc_size);
                self.last_alloc_size = required_size;
                self.allocations += 1;

                block.as_mut().top = NonNull::new_unchecked(top.as_ptr().add(required_size));

                NonNull::new_unchecked(addr_of_mut!((*tgt).value))
            }
        }
    }

    pub unsafe fn pop_deallocate(&mut self) {
        let mut block = self.block.unwrap();

        if Self::block_empty(block) {
            // block already empty, free from the previous block.
            let mut prev_block = block.as_ref().previous.unwrap();

            let new_top = prev_block.as_ref().top.as_ptr().sub(self.last_alloc_size);
            self.last_alloc_size = new_top.cast::<usize>().read();

            if new_top
                == prev_block
                    .cast::<u8>()
                    .as_ptr()
                    .add(std::mem::size_of::<BlockHeader>())
            {
                // old block is empty, deallocate.
                block.as_mut().previous = prev_block.as_ref().previous;
                let layout = Layout::from_size_align(prev_block.as_ref().size, 8).unwrap();
                std::alloc::dealloc(prev_block.as_ptr().cast(), layout);
                self.allocations -= 1;
                return;
            }

            prev_block.as_mut().top = NonNull::new_unchecked(new_top);
            self.allocations -= 1;
            return;
        }

        let new_top = block.as_ref().top.as_ptr().sub(self.last_alloc_size);
        self.last_alloc_size = new_top.cast::<usize>().read();
        self.allocations -= 1;
        block.as_mut().top = NonNull::new_unchecked(new_top);
    }

    pub fn allocations(&self) -> usize {
        self.allocations
    }
}

impl Drop for StackAllocator {
    fn drop(&mut self) {
        let mut cur = self.block;
        while let Some(b) = cur {
            unsafe {
                cur = b.as_ref().previous;
                let layout = Layout::from_size_align(b.as_ref().size, 8).unwrap();
                std::alloc::dealloc(b.as_ptr().cast(), layout);
            }
        }
    }
}

#[cfg(test)]
mod test {
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
                let alloc = alloc.alloc::<usize>();
                alloc.as_ptr().write(i);
                assert!(!allocations.contains(&alloc));
                allocations.push(alloc);
            }

            for (i, v) in allocations.iter().enumerate() {
                assert_eq!(i, v.as_ptr().read())
            }

            for _ in 0..(amount / 2) {
                allocations.pop();
                alloc.pop_deallocate();
            }

            let mut allocations_2 = Vec::new();

            for i in 0..amount {
                let alloc = alloc.alloc::<u128>();
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
