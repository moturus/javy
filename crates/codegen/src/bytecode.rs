use crate::Plugin;
use anyhow::Result;
pub(crate) fn compile_source(plugin: &Plugin, source: &[u8]) -> Result<Vec<u8>> {
    javy_motor_engine::compile_source(plugin.as_bytes(), source)
}
