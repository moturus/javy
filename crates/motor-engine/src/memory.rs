//! Motor reservations own their mappings; untouched pages read as the kernel's zeros.
use wasmi::{MemoryAllocator, MemoryBuffer, errors::MemoryError};

/// Largest capacity of one linear memory: a 32-bit memory's whole range.
pub const MAX_MEMORY_BYTES: usize = 4 << 30;

/// Backs each memory with one sparse lazy reservation of its whole capacity,
/// so growth never copies and only touched pages use RAM.
#[derive(Debug)]
pub struct Reservations {
    per_memory: usize,
}

impl Default for Reservations {
    fn default() -> Self {
        Self::new(MAX_MEMORY_BYTES)
    }
}

impl Reservations {
    pub fn new(per_memory: usize) -> Self {
        Self { per_memory }
    }
}

#[derive(Debug)]
struct Reservation {
    address: u64,
    capacity: usize,
}

// SysMem returns exclusive, stable, zeroed storage; only Wasmi borrows bytes.
unsafe impl MemoryBuffer for Reservation {
    fn data_ptr(&self) -> *mut u8 {
        self.address as *mut u8
    }
    fn capacity(&self) -> usize {
        self.capacity
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        moto_sys::SysMem::unmap(moto_sys::SysHandle::SELF, 0, u64::MAX, self.address)
            .expect("release Wasmi memory reservation");
    }
}

impl MemoryAllocator for Reservations {
    fn allocate(
        &self,
        initial: usize,
        maximum: Option<usize>,
    ) -> Result<Box<dyn MemoryBuffer>, MemoryError> {
        let capacity = maximum.unwrap_or(self.per_memory).min(self.per_memory);
        if initial > capacity || capacity > isize::MAX as usize {
            return Err(MemoryError::ResourceLimiterDeniedAllocation);
        }
        let pages = capacity.max(1).div_ceil(4096) as u64;
        let address = moto_sys::SysMem::map(
            moto_sys::SysHandle::SELF,
            moto_sys::SysMem::F_READABLE | moto_sys::SysMem::F_WRITABLE | moto_sys::SysMem::F_LAZY,
            u64::MAX,
            u64::MAX,
            moto_sys::sys_mem::PAGE_SIZE_SMALL,
            pages,
        )
        .map_err(|_| MemoryError::OutOfSystemMemory)?;
        Ok(Box::new(Reservation { address, capacity }))
    }
}
