use anyhow::{Context, Result};
use javy_motor_engine::{Host, Vm};
use wasmi::{Linker, Module};

fn main() -> std::process::ExitCode {
    match run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            if let Some(status) = error
                .downcast_ref::<wasmi::Error>()
                .and_then(|e| e.i32_exit_status())
            {
                return std::process::ExitCode::from(status as u8);
            }
            eprintln!("{error:#}");
            std::process::ExitCode::FAILURE
        }
    }
}

fn run() -> Result<()> {
    let args: Vec<_> = std::env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--help") {
        println!(
            "wasmi MODULE [--plugin PATH] [--namespace NAME] [--invoke EXPORT] [--fuel AMOUNT] [--stats]"
        );
        return Ok(());
    }
    if args.get(1).is_some_and(|arg| arg == "--version") {
        println!("wasmi 1.1.0 (Motor)");
        return Ok(());
    }
    javy_motor_engine::check_authority()?;
    let bytes = std::fs::read(args.get(1).context("expected module path")?)?;
    let mut plugin = None;
    let mut namespace = "javy-default-plugin-v4";
    let mut functions = Vec::new();
    let mut fuel = None;
    let mut stats = false;
    let mut options = args[2..].iter();
    while let Some(option) = options.next() {
        match option.as_str() {
            "--stats" => stats = true,
            "--fuel" => fuel = Some(options.next().context("expected fuel amount")?.parse()?),
            "--plugin" => plugin = Some(options.next().context("expected plugin path")?),
            "--namespace" => namespace = options.next().context("expected namespace")?,
            "--invoke" => {
                functions.push(options.next().context("expected function name")?.as_str())
            }
            _ => anyhow::bail!("unknown runner option {option}"),
        }
    }
    if functions.is_empty() {
        functions.push("_start");
    }
    let mut host = Host::default();
    host.input = Box::new(std::io::stdin());
    host.fuel = fuel;
    let start = std::time::Instant::now();
    let mut vm = if let Some(plugin) = plugin {
        let plugin = std::fs::read(plugin)?;
        let mut vm = Vm::new(&plugin, host)?;
        let engine = vm.store.engine();
        let module = Module::new(engine, &bytes)?;
        let mut linker = Linker::new(engine);
        linker.instance(&mut vm.store, namespace, vm.instance)?;
        javy_motor_engine::define_host_imports(&mut linker, &module, Some(namespace))?;
        vm.instance = linker.instantiate_and_start(&mut vm.store, &module)?;
        vm
    } else {
        Vm::new(&bytes, host)?
    };
    if stats {
        eprintln!(
            "instantiated in {:?}; fuel remaining {:?}",
            start.elapsed(),
            vm.store.get_fuel().ok()
        );
    }
    for function in functions {
        let start = std::time::Instant::now();
        let result = vm
            .instance
            .get_typed_func::<(), ()>(&vm.store, function)?
            .call(&mut vm.store, ());
        if stats {
            eprintln!(
                "{function}: {:?}; fuel remaining {:?}",
                start.elapsed(),
                vm.store.get_fuel().ok()
            );
        }
        // proc_exit ends the guest whatever its status; later invocations do not run.
        match result {
            Err(e) if e.i32_exit_status() == Some(0) => return Ok(()),
            result => result?,
        }
    }
    Ok(())
}
