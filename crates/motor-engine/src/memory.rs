//! Motor reservations own their mappings; Wasmi initializes only visible bytes.
use std::sync::{Arc, Mutex, OnceLock};
use wasmi::{MemoryAllocator, MemoryBuffer, errors::MemoryError};

pub const MAX_MEMORY_BYTES: usize = 96 << 20;
pub const MAX_RESERVED_BYTES: usize = 128 << 20;
pub const MAX_MEMORIES: usize = 4;

#[derive(Debug, Default)]
struct Usage {
    bytes: usize,
    memories: usize,
}

#[derive(Debug)]
pub struct Reservations {
    per_memory: usize,
    aggregate: usize,
    count: usize,
    usage: Arc<Mutex<Usage>>,
}

impl Reservations {
    pub fn new(per_memory: usize, aggregate: usize, count: usize) -> Self {
        Self {
            per_memory,
            aggregate,
            count,
            usage: Arc::default(),
        }
    }

    pub fn usage(&self) -> (usize, usize) {
        let usage = self.usage.lock().unwrap();
        (usage.bytes, usage.memories)
    }

    pub fn shared() -> Arc<Self> {
        static ALLOCATOR: OnceLock<Arc<Reservations>> = OnceLock::new();
        Arc::clone(ALLOCATOR.get_or_init(|| {
            Arc::new(Self::new(
                MAX_MEMORY_BYTES,
                MAX_RESERVED_BYTES,
                MAX_MEMORIES,
            ))
        }))
    }
}

#[derive(Debug)]
struct Reservation {
    address: Option<u64>,
    capacity: usize,
    mapped: usize,
    usage: Arc<Mutex<Usage>>,
}

// SysMem returns exclusive, stable writable storage; only Wasmi borrows bytes.
unsafe impl MemoryBuffer for Reservation {
    fn data_ptr(&self) -> *mut u8 {
        self.address.unwrap() as *mut u8
    }
    fn capacity(&self) -> usize {
        self.capacity
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if let Some(address) = self.address {
            moto_sys::SysMem::unmap(moto_sys::SysHandle::SELF, 0, u64::MAX, address)
                .expect("release Wasmi memory reservation");
        }
        let mut usage = self.usage.lock().unwrap();
        usage.bytes -= self.mapped;
        usage.memories -= 1;
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
        let mapped = capacity
            .max(1)
            .checked_add(4095)
            .ok_or(MemoryError::OutOfSystemMemory)?
            & !4095;
        {
            let mut usage = self.usage.lock().unwrap();
            let bytes = usage
                .bytes
                .checked_add(mapped)
                .ok_or(MemoryError::OutOfSystemMemory)?;
            if bytes > self.aggregate || usage.memories >= self.count {
                return Err(MemoryError::ResourceLimiterDeniedAllocation);
            }
            usage.bytes = bytes;
            usage.memories += 1;
        }
        // The lease rolls back admission if mapping fails before an address exists.
        let mut reservation = Reservation {
            address: None,
            capacity,
            mapped,
            usage: Arc::clone(&self.usage),
        };
        reservation.address = Some(
            moto_sys::SysMem::map(
                moto_sys::SysHandle::SELF,
                moto_sys::SysMem::F_READABLE
                    | moto_sys::SysMem::F_WRITABLE
                    | moto_sys::SysMem::F_LAZY,
                u64::MAX,
                u64::MAX,
                moto_sys::sys_mem::PAGE_SIZE_SMALL,
                (mapped >> 12) as u64,
            )
            .map_err(|_| MemoryError::OutOfSystemMemory)?,
        );
        Ok(Box::new(reservation))
    }
}
