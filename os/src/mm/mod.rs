mod address;
mod frame_allocator;
mod heap_allocator;
mod page_table;
mod memory_set;

pub use address::{
    PhysAddr, PhysPageNum, StepByOne, VirtAddr, VirtPageNum, VPNRange,
};
pub use frame_allocator::{frame_alloc, FrameTracker};
pub use page_table::{
    translated_byte_buffer, PageTable, PageTableEntry, PTEFlags,
};
pub use memory_set::{
    remap_test, KERNEL_SPACE, MapPermission, MemorySet,
};

pub fn init() {
    heap_allocator::init_heap();
    frame_allocator::init_frame_allocator();
    KERNEL_SPACE.exclusive_access().activate();
}
