use anyhow::{Context, Result, bail};
use rand_core::{Rng, SeedableRng};
use rand_pcg::Pcg64Mcg;
use std::io::{Cursor, Read, Write};
use wasmi::{Caller, Engine, ExternType, FuncType, Instance, Linker, Memory, Module, Store, Val};
use wasmtime_wizer::{InstanceState, SnapshotVal, ValType, Wizer};

pub struct Host {
    /// Guest stdin, read only when the guest reads descriptor 0.
    pub input: Box<dyn Read + Send>,
    pub deterministic: bool,
    pub fuel: Option<u64>,
    rng: Option<Pcg64Mcg>,
    dummy_imports: bool,
    limits: wasmi::StoreLimits,
}

impl Default for Host {
    fn default() -> Self {
        Self {
            input: Box::new(Cursor::new(Vec::new())),
            deterministic: false,
            fuel: None,
            rng: None,
            dummy_imports: false,
            limits: wasmi::StoreLimitsBuilder::new()
                .memory_size(96 << 20)
                .memories(4)
                .instances(64)
                .tables(16)
                .table_elements(32_768)
                .build(),
        }
    }
}

pub struct Vm {
    pub store: Store<Host>,
    pub instance: Instance,
}

impl Vm {
    pub fn new(bytes: &[u8], host: Host) -> Result<Self> {
        let mut config = wasmi::Config::default();
        config.consume_fuel(host.fuel.is_some());
        #[cfg(target_os = "motor")]
        config.with_memory_allocator(crate::memory::Reservations::shared());
        let engine = Engine::new(&config);
        let module = Module::new(&engine, bytes)?;
        let mut store = Store::new(&engine, host);
        store.limiter(|host| &mut host.limits);
        if let Some(fuel) = store.data().fuel {
            store.set_fuel(fuel)?;
        }
        let mut linker = Linker::<Host>::new(&engine);
        define_host_imports(&mut linker, &module, None)?;
        let instance = linker.instantiate_and_start(&mut store, &module)?;
        Ok(Self { store, instance })
    }

    pub fn memory(&self) -> Result<Memory> {
        self.instance
            .get_memory(&self.store, "memory")
            .context("missing memory")
    }

    pub fn call(&mut self, name: &str) -> Result<()> {
        let f = self.instance.get_typed_func::<(), ()>(&self.store, name)?;
        match f.call(&mut self.store, ()) {
            Ok(()) => Ok(()),
            Err(e) if e.i32_exit_status() == Some(0) => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

pub fn compile_source(plugin: &[u8], source: &[u8]) -> Result<Vec<u8>> {
    let mut vm = Vm::new(
        plugin,
        Host {
            dummy_imports: true,
            ..Host::default()
        },
    )?;
    let memory = vm.memory()?;
    let size: i32 = source.len().try_into()?;
    let alloc = vm
        .instance
        .get_typed_func::<(i32, i32, i32, i32), i32>(&vm.store, "cabi_realloc")?;
    let ptr = alloc.call(&mut vm.store, (0, 0, 1, size))? as u32;
    memory.write(&mut vm.store, ptr as usize, source)?;
    let compile = vm
        .instance
        .get_typed_func::<(i32, i32), i32>(&vm.store, "compile-src")?;
    let ret = compile.call(&mut vm.store, (ptr as i32, size))? as u32;
    let mut header = [0; 12];
    memory.read(&vm.store, ret as usize, &mut header)?;
    let status = u32::from_le_bytes(header[..4].try_into()?);
    let ptr = u32::from_le_bytes(header[4..8].try_into()?);
    let len = u32::from_le_bytes(header[8..].try_into()?);
    let data = memory.data(&vm.store);
    let range = (ptr as usize)
        ..(ptr as usize)
            .checked_add(len as usize)
            .context("bytecode range overflow")?;
    let bytes = data
        .get(range)
        .context("bytecode result out of bounds")?
        .to_vec();
    if status != 0 {
        bail!("JS compilation failed: {}", String::from_utf8_lossy(&bytes));
    }
    Ok(bytes)
}

pub fn config_schema(plugin: &[u8]) -> Result<Vec<u8>> {
    let mut vm = Vm::new(
        plugin,
        Host {
            dummy_imports: true,
            ..Host::default()
        },
    )?;
    let f = vm
        .instance
        .get_typed_func::<(), i32>(&vm.store, "config-schema")?;
    let ptr = f.call(&mut vm.store, ())? as u32;
    let memory = vm.memory()?;
    let mut header = [0; 8];
    memory.read(&vm.store, ptr as usize, &mut header)?;
    let ptr = u32::from_le_bytes(header[..4].try_into()?);
    let len = u32::from_le_bytes(header[4..].try_into()?);
    let end = (ptr as usize)
        .checked_add(len as usize)
        .context("schema range overflow")?;
    Ok(memory
        .data(&vm.store)
        .get(ptr as usize..end)
        .context("schema out of bounds")?
        .to_vec())
}

pub async fn initialize_named(
    bytes: &[u8],
    input: Vec<u8>,
    deterministic: bool,
    keep_init: bool,
    init_name: &str,
) -> Result<Vec<u8>> {
    let mut wizer = Wizer::new();
    wizer.init_func(init_name).keep_init_func(keep_init);
    let (context, instrumented) = wizer.instrument(bytes)?;
    let mut vm = Vm::new(
        &instrumented,
        Host {
            input: Box::new(Cursor::new(input)),
            deterministic,
            ..Host::default()
        },
    )?;
    if vm.instance.get_func(&vm.store, "_initialize").is_some() {
        vm.call("_initialize")?;
    }
    vm.call(init_name).context("plugin initializer trapped")?;
    wizer.snapshot(&context, &mut vm).await
}

impl InstanceState for Vm {
    async fn global_get(&mut self, name: &str, _: ValType) -> SnapshotVal {
        match self
            .instance
            .get_global(&self.store, name)
            .expect("instrumented global")
            .get(&self.store)
        {
            Val::I32(x) => SnapshotVal::I32(x),
            Val::I64(x) => SnapshotVal::I64(x),
            Val::F32(x) => SnapshotVal::F32(x.to_bits()),
            Val::F64(x) => SnapshotVal::F64(x.to_bits()),
            Val::V128(x) => SnapshotVal::V128(x.as_u128()),
            _ => panic!("unsupported snapshot global"),
        }
    }

    async fn memory_contents(&mut self, name: &str, contents: impl FnOnce(&[u8]) + Send) {
        let memory = self
            .instance
            .get_memory(&self.store, name)
            .expect("instrumented memory");
        contents(memory.data(&self.store));
    }
}

/// Bytes one `fd_read` moves at most; a short read makes the guest ask again.
const READ_CHUNK: usize = 64 << 10;

fn arg(args: &[Val], n: usize) -> Result<usize, wasmi::Error> {
    args.get(n)
        .and_then(Val::i32)
        .map(|v| v as u32 as usize)
        .ok_or_else(|| wasmi::Error::new("WASI argument is not an i32"))
}

fn read_input(host: &mut Host, bytes: &mut [u8]) -> std::io::Result<usize> {
    loop {
        match host.input.read(bytes) {
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            result => return result,
        }
    }
}

fn wasi(caller: &mut Caller<'_, Host>, name: &str, args: &[Val]) -> Result<i32, wasmi::Error> {
    let p = |n: usize| arg(args, n);
    if name == "proc_exit" {
        return Err(wasmi::Error::i32_exit(p(0)? as i32));
    }
    let memory = caller
        .get_export("memory")
        .and_then(|x| x.into_memory())
        .ok_or_else(|| wasmi::Error::new("missing caller memory"))?;
    let err = |e: wasmi::errors::MemoryError| wasmi::Error::new(e.to_string());
    match name {
        "environ_sizes_get" | "args_sizes_get" => {
            memory.write(&mut *caller, p(0)?, &[0; 4]).map_err(err)?;
            memory.write(&mut *caller, p(1)?, &[0; 4]).map_err(err)?;
        }
        "environ_get" | "args_get" => {}
        "fd_close" => return Ok(if p(0)? <= 2 { 0 } else { 8 }),
        "fd_seek" => return Ok(70),
        "fd_fdstat_get" => {
            if p(0)? > 2 {
                return Ok(8);
            }
            let mut stat = [0; 24];
            stat[0] = 2;
            let rights: u64 = if p(0)? == 0 { 2 } else { 64 };
            stat[8..16].copy_from_slice(&rights.to_le_bytes());
            memory.write(&mut *caller, p(1)?, &stat).map_err(err)?;
        }
        "fd_read" | "fd_write" => {
            let fd = p(0)?;
            if (name == "fd_read" && fd != 0) || (name == "fd_write" && !(1..=2).contains(&fd)) {
                return Ok(8);
            }
            let mut total = 0u32;
            for i in 0..p(2)? {
                let mut io = [0; 8];
                memory
                    .read(
                        &*caller,
                        p(1)?
                            .checked_add(
                                i.checked_mul(8)
                                    .ok_or_else(|| wasmi::Error::new("iov overflow"))?,
                            )
                            .ok_or_else(|| wasmi::Error::new("iov overflow"))?,
                        &mut io,
                    )
                    .map_err(err)?;
                let ptr = u32::from_le_bytes(io[..4].try_into().unwrap()) as usize;
                let len = u32::from_le_bytes(io[4..].try_into().unwrap()) as usize;
                let end = ptr
                    .checked_add(len)
                    .ok_or_else(|| wasmi::Error::new("iov range overflow"))?;
                if end > memory.data_size(&*caller) {
                    return Ok(21);
                }
                let count = if name == "fd_write" {
                    let bytes = &memory.data(&*caller)[ptr..end];
                    let result = if fd == 1 {
                        std::io::stdout().write_all(bytes)
                    } else {
                        std::io::stderr().write_all(bytes)
                    };
                    result.map_err(|e| wasmi::Error::new(e.to_string()))?;
                    len
                } else {
                    let mut bytes = vec![0; len.min(READ_CHUNK)];
                    let count = read_input(caller.data_mut(), &mut bytes)
                        .map_err(|e| wasmi::Error::new(e.to_string()))?;
                    memory
                        .write(&mut *caller, ptr, &bytes[..count])
                        .map_err(err)?;
                    count
                };
                total = total
                    .checked_add(count as u32)
                    .ok_or_else(|| wasmi::Error::new("iov total overflow"))?;
                if name == "fd_read" && count < len {
                    break;
                }
            }
            memory
                .write(&mut *caller, p(3)?, &total.to_le_bytes())
                .map_err(err)?;
        }
        "clock_time_get" => {
            let nanos = if caller.data().deterministic {
                0
            } else {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|e| wasmi::Error::new(e.to_string()))?
                    .as_nanos() as u64
            };
            memory
                .write(&mut *caller, p(2)?, &nanos.to_le_bytes())
                .map_err(err)?;
        }
        "random_get" => {
            let end = p(0)?
                .checked_add(p(1)?)
                .ok_or_else(|| wasmi::Error::new("random range overflow"))?;
            if end > memory.data_size(&*caller) {
                return Ok(21);
            }
            let mut bytes = vec![0; p(1)?];
            if caller.data().deterministic {
                caller
                    .data_mut()
                    .rng
                    .get_or_insert_with(|| Pcg64Mcg::seed_from_u64(42))
                    .fill_bytes(&mut bytes);
            } else {
                fill_random(&mut bytes).map_err(|e| wasmi::Error::new(e.to_string()))?;
            }
            memory.write(&mut *caller, p(0)?, &bytes).map_err(err)?;
        }
        _ => {
            return Err(wasmi::Error::new(format!(
                "WASI import not implemented: {name}"
            )));
        }
    }
    Ok(0)
}

fn fill_random(bytes: &mut [u8]) -> Result<()> {
    #[cfg(target_os = "motor")]
    moto_rt::fill_random_bytes(bytes);
    #[cfg(not(target_os = "motor"))]
    getrandom::fill(bytes).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    Ok(())
}

/// Types of the implemented preview1 functions; other names link and fail when called.
fn wasi_signature(name: &str) -> Option<FuncType> {
    use wasmi::ValType::{I32, I64};
    let (params, results): (&[wasmi::ValType], &[wasmi::ValType]) = match name {
        "proc_exit" => (&[I32], &[]),
        "fd_close" => (&[I32], &[I32]),
        "environ_sizes_get" | "args_sizes_get" | "environ_get" | "args_get" | "fd_fdstat_get"
        | "random_get" => (&[I32, I32], &[I32]),
        "fd_seek" => (&[I32, I64, I32, I32], &[I32]),
        "fd_read" | "fd_write" => (&[I32, I32, I32, I32], &[I32]),
        "clock_time_get" => (&[I32, I64, I32], &[I32]),
        _ => return None,
    };
    Some(FuncType::new(
        params.iter().copied(),
        results.iter().copied(),
    ))
}

pub fn define_host_imports(
    linker: &mut Linker<Host>,
    module: &Module,
    skip_namespace: Option<&str>,
) -> Result<()> {
    let mut defined = std::collections::HashSet::new();
    for import in module.imports() {
        if skip_namespace == Some(import.module()) {
            continue;
        }
        let ExternType::Func(ty) = import.ty() else {
            bail!("unsupported import {}::{}", import.module(), import.name());
        };
        if let Some(expected) = wasi_signature(import.name())
            .filter(|_| import.module() == "wasi_snapshot_preview1")
            .filter(|expected| expected != ty)
        {
            bail!(
                "import wasi_snapshot_preview1::{} has type {:?} -> {:?}, expected {:?} -> {:?}",
                import.name(),
                ty.params(),
                ty.results(),
                expected.params(),
                expected.results()
            );
        }
        // A module may import one function several times; define it once.
        if !defined.insert((import.module(), import.name())) {
            continue;
        }
        let namespace = import.module().to_owned();
        let name = import.name().to_owned();
        let results = ty.results().to_vec();
        linker.func_new(
            import.module(),
            import.name(),
            ty.clone(),
            move |mut caller, args, out| {
                for (v, ty) in out.iter_mut().zip(&results) {
                    *v = Val::default(*ty);
                }
                if caller.data().dummy_imports {
                    return Ok(());
                }
                if namespace != "wasi_snapshot_preview1" {
                    return Err(wasmi::Error::new(format!(
                        "unsupported import {namespace}::{name}"
                    )));
                }
                let errno = wasi(&mut caller, &name, args)?;
                if let Some(v) = out.first_mut() {
                    *v = Val::I32(errno);
                }
                Ok(())
            },
        )?;
    }
    Ok(())
}
