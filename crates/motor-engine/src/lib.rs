use std::future::Future;
use std::task::{Context, Poll, Waker};
mod wasmi_backend;
pub use wasmi_backend::{
    Host, Vm, compile_source, config_schema, define_host_imports, initialize_named,
};
#[cfg(target_os = "motor")]
pub mod memory;

pub async fn initialize(
    bytes: &[u8],
    input: Vec<u8>,
    deterministic: bool,
    keep_init: bool,
) -> anyhow::Result<Vec<u8>> {
    initialize_named(bytes, input, deterministic, keep_init, "initialize-runtime").await
}

// Compiler and snapshot host operations are synchronous; pending work is a bug.
pub fn block_on<T>(future: impl Future<Output = T>) -> T {
    let mut future = std::pin::pin!(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("unexpected asynchronous Javy compiler operation"),
    }
}
