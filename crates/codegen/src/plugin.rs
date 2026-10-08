use anyhow::{Result, bail};
use std::{borrow::Cow, collections::HashMap, fs, path::Path, str};
use wasmparser::{ExternalKind, FuncType, Parser, Payload, ValType, Validator, WasmFeatures};

/// A Javy plugin.
#[derive(Clone, Debug, Default)]
pub struct Plugin {
    bytes: Cow<'static, [u8]>,
}

impl Plugin {
    /// Constructs a new [`Plugin`].
    pub fn new(bytes: Cow<'static, [u8]>) -> Result<Self> {
        Self::validate(&bytes)?;
        Ok(Self { bytes })
    }

    /// Constructs a new [`Plugin`] from a given path.
    pub fn new_from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let bytes = fs::read(path)?;
        Self::new(bytes.into())
    }

    /// Returns the [`Plugin`] as bytes
    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Validates if `plugin_bytes` are a valid plugin.
    pub fn validate(plugin_bytes: &[u8]) -> Result<()> {
        if !Parser::is_core_wasm(plugin_bytes) {
            bail!("Could not process plugin: Expected Wasm module, received unknown file type");
        }

        let mut errors = vec![];

        // Validate with walrus's feature set but without building its IR, which
        // would hold the whole plugin's functions in memory.
        let mut features = WasmFeatures::default();
        features.insert(WasmFeatures::LEGACY_EXCEPTIONS | WasmFeatures::WIDE_ARITHMETIC);
        let types = Validator::new_with_features(features).validate_all(plugin_bytes)?;
        let types = types.as_ref();
        let mut exports = HashMap::new();
        let mut has_import_namespace = false;
        for payload in Parser::new(0).parse_all(plugin_bytes) {
            match payload? {
                Payload::ExportSection(reader) => {
                    for export in reader {
                        let export = export?;
                        exports.insert(export.name, (export.kind, export.index));
                    }
                }
                Payload::CustomSection(section) if section.name() == "import_namespace" => {
                    has_import_namespace = true;
                }
                _ => {}
            }
        }
        let exported_func = |name: &str| match exports.get(name) {
            Some((ExternalKind::Func, index)) => {
                Some(types[types.core_function_at(*index)].unwrap_func())
            }
            _ => None,
        };

        if exported_func("compile_src").is_some() {
            bail!("Could not process plugin: Using unsupported legacy plugin API");
        }

        if let Err(err) = validate_exported_func(
            exported_func("initialize-runtime"),
            "initialize-runtime",
            &[],
            &[],
        ) {
            errors.push(err);
        }
        if let Err(err) = validate_exported_func(
            exported_func("compile-src"),
            "compile-src",
            &[ValType::I32, ValType::I32],
            &[ValType::I32],
        ) {
            errors.push(err);
        }
        if let Err(err) = validate_exported_func(
            exported_func("invoke"),
            "invoke",
            &[
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
                ValType::I32,
            ],
            &[],
        ) {
            errors.push(err);
        }

        if !matches!(exports.get("memory"), Some((ExternalKind::Memory, _))) {
            errors.push("missing exported memory named `memory`".to_string());
        }

        if !has_import_namespace {
            errors.push("missing custom section named `import_namespace`".to_string());
        }

        if !errors.is_empty() {
            bail!("Could not process plugin: {}", errors.join(", "))
        }
        Ok(())
    }

    pub(crate) fn import_namespace(&self) -> Result<String> {
        for payload in Parser::new(0).parse_all(&self.bytes) {
            if let Payload::CustomSection(section) = payload?
                && section.name() == "import_namespace"
            {
                return Ok(str::from_utf8(section.data())?.to_string());
            }
        }
        bail!("Plugin is missing import_namespace custom section")
    }
}

fn validate_exported_func(
    ty: Option<&FuncType>,
    name: &str,
    expected_params: &[ValType],
    expected_results: &[ValType],
) -> Result<(), String> {
    let ty = ty.ok_or_else(|| format!("missing export for function named `{name}`"))?;
    let params = ty.params();
    let has_correct_params = params == expected_params;
    let results = ty.results();
    let has_correct_results = results == expected_results;
    if !has_correct_params || !has_correct_results {
        return Err(format!("type for function `{name}` is incorrect"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use anyhow::Result;
    use walrus::{FunctionBuilder, ModuleConfig, ValType};

    use crate::Plugin;

    #[test]
    fn test_validate_plugin_with_empty_file() -> Result<()> {
        let err = Plugin::new(vec![].into()).err().unwrap();
        assert_eq!(
            err.to_string(),
            "Could not process plugin: Expected Wasm module, received unknown file type"
        );
        Ok(())
    }

    #[test]
    fn test_validate_plugin_with_old_plugin() -> Result<()> {
        let mut module = walrus::Module::with_config(ModuleConfig::default());
        module.add_import_memory("foo", "memory", false, false, 0, None, None);
        let mut compile_src_fn = FunctionBuilder::new(
            &mut module.types,
            &[ValType::I32, ValType::I32],
            &[ValType::I32],
        );
        compile_src_fn.func_body().unreachable();
        let compile_src_fn = compile_src_fn.finish(vec![], &mut module.funcs);
        module.exports.add("compile_src", compile_src_fn);

        let err = Plugin::new(module.emit_wasm().into()).err().unwrap();
        assert_eq!(
            err.to_string(),
            "Could not process plugin: Using unsupported legacy plugin API"
        );
        Ok(())
    }

    #[test]
    fn test_validate_plugin_with_incorrect_invoke_and_everything_missing() -> Result<()> {
        let mut module = walrus::Module::with_config(ModuleConfig::default());
        let invoke = FunctionBuilder::new(
            &mut module.types,
            &[ValType::I32, ValType::I32, ValType::I32, ValType::I32],
            &[],
        )
        .finish(vec![], &mut module.funcs);
        module.exports.add("invoke", invoke);

        let plugin_bytes = module.emit_wasm();
        let error = Plugin::validate(&plugin_bytes).err().unwrap();
        assert_eq!(
            error.to_string(),
            "Could not process plugin: missing export for function named \
            `initialize-runtime`, missing export for function named \
            `compile-src`, type for function `invoke` is incorrect, missing \
            exported memory named `memory`, missing custom section named \
            `import_namespace`"
        );
        Ok(())
    }

    #[test]
    fn test_validate_plugin_with_everything_missing() -> Result<()> {
        let mut empty_module = walrus::Module::with_config(ModuleConfig::default());
        let plugin_bytes = empty_module.emit_wasm();
        let error = Plugin::new(plugin_bytes.into()).err().unwrap();
        assert_eq!(
            error.to_string(),
            "Could not process plugin: missing export for function named \
            `initialize-runtime`, missing export for function named \
            `compile-src`, missing export for function named `invoke`, \
            missing exported memory named `memory`, missing custom section \
            named `import_namespace`"
        );
        Ok(())
    }

    #[test]
    fn test_validate_plugin_with_wrong_params_for_initialize_runtime() -> Result<()> {
        let mut module = walrus::Module::with_config(ModuleConfig::default());
        let initialize_runtime = FunctionBuilder::new(&mut module.types, &[ValType::I32], &[])
            .finish(vec![], &mut module.funcs);
        module.exports.add("initialize-runtime", initialize_runtime);

        let plugin_bytes = module.emit_wasm();
        let error = Plugin::new(plugin_bytes.into()).err().unwrap();
        let expected_part_of_error =
            "Could not process plugin: type for function `initialize-runtime` is incorrect,";
        if !error.to_string().contains(expected_part_of_error) {
            panic!(
                "Expected error to contain '{expected_part_of_error}' but it did not. Full error is: '{error}'"
            );
        }
        Ok(())
    }

    #[test]
    fn test_validate_plugin_with_wrong_results_for_initialize_runtime() -> Result<()> {
        let mut module = walrus::Module::with_config(ModuleConfig::default());
        let mut initialize_runtime = FunctionBuilder::new(&mut module.types, &[], &[ValType::I32]);
        initialize_runtime.func_body().i32_const(0);
        let initialize_runtime = initialize_runtime.finish(vec![], &mut module.funcs);
        module.exports.add("initialize-runtime", initialize_runtime);

        let plugin_bytes = module.emit_wasm();
        let error = Plugin::new(plugin_bytes.into()).err().unwrap();
        let expected_part_of_error =
            "Could not process plugin: type for function `initialize-runtime` is incorrect,";
        if !error.to_string().contains(expected_part_of_error) {
            panic!(
                "Expected error to contain '{expected_part_of_error}' but it did not. Full error is: '{error}'"
            );
        }
        Ok(())
    }
}
