use motor_javy_cxx_tls::{__wrap___cxa_thread_atexit, __wrap___emutls_get_address, Control};
use std::sync::atomic::{AtomicUsize, Ordering};

#[repr(C)]
struct Record {
    size: usize,
    align: usize,
    index: usize,
    initial: *const u8,
}
// Immutable ABI records; the Javy-local shim only reads them.
unsafe impl Sync for Record {}
static INITIAL: [u8; 32] = [9; 32];
static RECORD: Record = Record {
    size: 32,
    align: 256,
    index: 0,
    initial: INITIAL.as_ptr(),
};
static ZERO: Record = Record {
    size: 64,
    align: 64,
    index: 0,
    initial: std::ptr::null(),
};
static NEXT: AtomicUsize = AtomicUsize::new(128);
static EXTRA: AtomicUsize = AtomicUsize::new(0);

fn address(record: &'static Record) -> *mut u8 {
    unsafe { __wrap___emutls_get_address((record as *const Record).cast_mut().cast::<Control>()) }
}

unsafe extern "C" fn extra(_: *mut u8) {
    EXTRA.fetch_add(1, Ordering::Relaxed);
}

unsafe extern "C" fn destructor(object: *mut u8) {
    let index = unsafe { *Box::from_raw(object.cast::<usize>()) };
    assert_eq!(NEXT.fetch_sub(1, Ordering::Relaxed) - 1, index);
    // The state and all slots must remain available while destructors execute.
    assert_eq!(unsafe { *address(&RECORD) }, 17);
    if index == 127 {
        assert_eq!(
            unsafe {
                __wrap___cxa_thread_atexit(extra, std::ptr::null_mut(), std::ptr::null_mut())
            },
            0
        );
    }
}

pub fn run() {
    for _ in 0..8 {
        NEXT.store(128, Ordering::Relaxed);
        EXTRA.store(0, Ordering::Relaxed);
        std::thread::spawn(|| {
            assert_eq!(
                std::mem::size_of::<Record>(),
                std::mem::size_of::<Control>()
            );
            let slot = address(&RECORD);
            assert_eq!(slot as usize % 256, 0);
            assert_eq!(unsafe { std::slice::from_raw_parts(slot, 32) }, &INITIAL);
            unsafe {
                slot.write(17);
            }
            assert_eq!(address(&RECORD), slot);
            let zero = address(&ZERO);
            assert_eq!(zero as usize % 64, 0);
            assert_eq!(unsafe { std::slice::from_raw_parts(zero, 64) }, &[0; 64]);
            for index in 0_usize..128 {
                let object = Box::into_raw(Box::new(index)).cast::<u8>();
                assert_eq!(
                    unsafe { __wrap___cxa_thread_atexit(destructor, object, std::ptr::null_mut()) },
                    0
                );
            }
        })
        .join()
        .unwrap();
        assert_eq!(NEXT.load(Ordering::Relaxed), 0);
        assert_eq!(EXTRA.load(Ordering::Relaxed), 1);
    }
    println!("TLS PASS destructors=128 reentrant=1 alignment=256 threads=8");
}
