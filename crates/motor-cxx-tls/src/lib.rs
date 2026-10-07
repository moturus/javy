#![cfg(target_os = "motor")]
//! Javy-local replacement for the two C++ TLS entry points.
//! Link with --wrap=__emutls_get_address and --wrap=__cxa_thread_atexit.
//! One key owns both the destructor stack and the backing allocations.
use std::alloc::{Layout, alloc, alloc_zeroed, dealloc, handle_alloc_error};
use std::collections::BTreeMap;
use std::sync::OnceLock;

type Destructor = unsafe extern "C" fn(*mut u8);
#[repr(C)]
pub struct Control {
    size: usize,
    align: usize,
    index: usize,
    initial: *const u8,
}
struct Slot {
    pointer: *mut u8,
    layout: Layout,
}
impl Drop for Slot {
    fn drop(&mut self) {
        unsafe { dealloc(self.pointer, self.layout) };
    }
}
#[derive(Default)]
struct State {
    slots: BTreeMap<usize, Slot>,
    destructors: Vec<(Destructor, *mut u8)>,
}
static KEY: OnceLock<usize> = OnceLock::new();
fn key() -> usize {
    *KEY.get_or_init(|| moto_rt::tls::create(Some(cleanup)))
}
fn state() -> *mut State {
    let key = key();
    let pointer = unsafe { moto_rt::tls::get(key) } as *mut State;
    if !pointer.is_null() {
        return pointer;
    }
    let pointer = Box::into_raw(Box::new(State::default()));
    unsafe { moto_rt::tls::set(key, pointer.cast()) };
    pointer
}
unsafe extern "C" fn cleanup(pointer: *mut u8) {
    let pointer = pointer.cast::<State>();
    // The runtime clears the key before calling us. Keep it available while
    // user destructors run: they may access TLS or register more destructors.
    unsafe { moto_rt::tls::set(key(), pointer.cast()) };
    loop {
        let next = unsafe { (*pointer).destructors.pop() };
        let Some((destructor, object)) = next else {
            break;
        };
        unsafe { destructor(object) };
    }
    unsafe { moto_rt::tls::set(key(), std::ptr::null_mut()) };
    // No user destructor can observe released TLS storage.
    unsafe { drop(Box::from_raw(pointer)) };
}

/// # Safety
/// `control` must be a compiler-generated emulated TLS control record.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __wrap___emutls_get_address(control: *mut Control) -> *mut u8 {
    let slots = unsafe { &mut (*state()).slots };
    let slot = slots.entry(control as usize).or_insert_with(|| {
        let control = unsafe { &*control };
        let layout = Layout::from_size_align(control.size.max(1), control.align.max(1))
            .expect("invalid compiler TLS layout");
        let pointer = unsafe {
            if control.initial.is_null() {
                alloc_zeroed(layout)
            } else {
                alloc(layout)
            }
        };
        if pointer.is_null() {
            handle_alloc_error(layout);
        }
        if !control.initial.is_null() {
            unsafe { std::ptr::copy_nonoverlapping(control.initial, pointer, control.size) };
        }
        Slot { pointer, layout }
    });
    slot.pointer
}

/// # Safety
/// `destructor(object)` must be valid until this thread's exit.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __wrap___cxa_thread_atexit(
    destructor: Destructor,
    object: *mut u8,
    _dso: *mut u8,
) -> i32 {
    unsafe { (*state()).destructors.push((destructor, object)) };
    0
}
