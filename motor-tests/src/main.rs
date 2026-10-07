#[cfg(target_os = "motor")]
mod tls;
#[cfg(target_os = "motor")]
mod motor {
    use anyhow::{Result, ensure};
    use javy_motor_engine::memory::Reservations;
    use moto_sys::{SysMem, SysRay, stats::MemoryStats};
    use std::sync::Arc;
    use wasmi::{Config, Engine, Linker, Memory, MemoryAllocator, MemoryType, Module, Store};

    fn charge() -> u64 {
        let mut entries = [moto_sys::stats::MetricEntry::default(); 128];
        let (n, total) = SysRay::query_stats(moto_sys::current_pid(), &mut entries).unwrap();
        assert_eq!(n, total);
        entries[..n].iter().find(|e| e.metric == 0).unwrap().value
    }

    pub fn backing() -> Result<()> {
        let allocator = Arc::new(Reservations::new(96 << 20, 128 << 20, 4));
        let mut config = Config::default();
        config.with_memory_allocator(allocator.clone());
        let engine = Engine::new(&config);
        let before = MemoryStats::get().unwrap().used();
        let mut store = Store::new(&engine, ());
        let memory = Memory::new(&mut store, MemoryType::new(0, Some(1536)))?;
        let pointer = memory.data_ptr(&store);
        ensure!(memory.size(&store) == 0 && memory.data_size(&store) == 0);
        ensure!(allocator.usage() == (96 << 20, 1));
        ensure!(
            SysMem::virt_to_phys(pointer as u64).is_err(),
            "untouched reservation faulted eagerly"
        );
        let reserved = MemoryStats::get().unwrap().used().saturating_sub(before);
        ensure!(
            reserved < 16 << 20,
            "lazy reservation used {reserved} physical bytes"
        );
        memory.grow(&mut store, 256)?;
        let touched = MemoryStats::get().unwrap().used().saturating_sub(before);
        ensure!(touched >= 16 << 20, "growth did not commit visible pages");
        ensure!(memory.data_ptr(&store) == pointer && memory.data(&store).iter().all(|b| *b == 0));
        memory.write(&mut store, 0, b"preserved")?;
        memory.grow(&mut store, 1)?;
        ensure!(&memory.data(&store)[..9] == b"preserved");
        ensure!(memory.data(&store)[16 << 20..].iter().all(|b| *b == 0));
        ensure!(memory.grow(&mut store, 1536).is_err());
        ensure!(memory.ty(&store).maximum() == Some(1536));
        drop(store);
        ensure!(allocator.usage() == (0, 0));
        println!("lazy backing reserve=100663296 physical={reserved} touched={touched}");

        let allocator = Arc::new(Reservations::new(2 << 16, 4 << 16, 2));
        let mut config = Config::default();
        config.with_memory_allocator(allocator.clone());
        let engine = Engine::new(&config);
        let mut store = Store::new(&engine, ());
        ensure!(Memory::new(&mut store, MemoryType::new(3, None)).is_err());
        ensure!(allocator.usage() == (0, 0));
        let imported = Memory::new(&mut store, MemoryType::new(1, Some(2)))?;
        let module = Module::new(
            &engine,
            br#"(module
          (import "host" "memory" (memory 1 2))
          (memory (export "second") 1 2)
          (func (export "grow") (result i32) i32.const 2 memory.grow 0))"#,
        )?;
        let mut linker = Linker::new(&engine);
        linker.define("host", "memory", imported)?;
        let instance = linker.instantiate_and_start(&mut store, &module)?;
        ensure!(instance.get_memory(&store, "second").unwrap().size(&store) == 1);
        let grow = instance.get_typed_func::<(), i32>(&store, "grow")?;
        ensure!(grow.call(&mut store, ())? == -1 && imported.size(&store) == 1);
        ensure!(Memory::new(&mut store, MemoryType::new(0, Some(1))).is_err());
        drop(store);
        ensure!(allocator.usage() == (0, 0));
        // Real static storage retains the upstream caller-owned API semantics.
        static mut STATIC: [u8; 2 << 16] = [7; 2 << 16];
        let mut store = Store::new(&engine, ());
        let memory = Memory::new_static(&mut store, MemoryType::new(1, Some(2)), unsafe {
            &mut *std::ptr::addr_of_mut!(STATIC)
        })?;
        ensure!(allocator.usage() == (0, 0) && memory.data(&store).iter().all(|b| *b == 0));
        memory.grow(&mut store, 1)?;
        ensure!(memory.data(&store).iter().all(|b| *b == 0));
        drop(store);
        let malformed = Module::new(
            &engine,
            br#"(module (memory 1 2)
          (data (i32.const 65535) "too long"))"#,
        )?;
        // Warm Store allocations, then verify actual mappings and process charge.
        let mut warm_charge = None;
        for _ in 0..1025 {
            let mut store = Store::new(&engine, ());
            ensure!(
                Linker::new(&engine)
                    .instantiate_and_start(&mut store, &malformed)
                    .is_err()
            );
            drop(store);
            ensure!(allocator.usage() == (0, 0));
            let mut store = Store::new(&engine, ());
            let memory = Memory::new(&mut store, MemoryType::new(1, Some(2)))?;
            let address = memory.data_ptr(&store) as u64;
            memory.grow(&mut store, 1)?;
            ensure!(memory.data(&store).iter().all(|b| *b == 0));
            drop(store);
            ensure!(allocator.usage() == (0, 0));
            ensure!(
                SysMem::virt_to_phys(address).is_err(),
                "mapping survived Store drop"
            );
            let current = charge();
            if let Some(warm) = warm_charge {
                ensure!(
                    current == warm,
                    "Store teardown charge grew: {warm} -> {current}"
                );
            } else {
                warm_charge = Some(current);
            }
        }
        // Zero-memory and aggregate refusal are distinct from the count limit.
        let allocator = Reservations::new(2 << 16, 2 << 16, 4);
        let first = allocator.allocate(0, Some(2 << 16))?;
        ensure!(allocator.allocate(0, Some(1 << 16)).is_err());
        drop(first);
        ensure!(allocator.usage() == (0, 0));
        println!(
            "backing PASS defined imported multi static zero grow limits failed-init teardown=1024 charge={}",
            warm_charge.unwrap()
        );
        Ok(())
    }

    pub fn fault() -> Result<()> {
        let allocator = Reservations::new(96 << 20, 96 << 20, 1);
        let memory = allocator.allocate(0, Some(96 << 20))?;
        // Fill physical memory while leaving a fixed lazy reservation unfaulted.
        // Mapping admission and fault-time exhaustion are separate contracts.
        let stats = MemoryStats::get().unwrap();
        let keep_free = 48 << 20;
        let hold = (stats.available - stats.used()).saturating_sub(keep_free) as usize;
        let mut heap = Vec::new();
        heap.try_reserve_exact(hold)?;
        heap.resize(hold, 1_u8);
        println!("fault-reservation-ready reserve=100663296 held={hold}");
        for offset in (0..memory.capacity()).step_by(4096) {
            unsafe {
                memory.data_ptr().add(offset).write_volatile(1);
            }
        }
        std::hint::black_box(heap);
        anyhow::bail!("fault exhaustion probe unexpectedly completed")
    }
}

fn main() -> anyhow::Result<()> {
    javy_motor_engine::check_authority()?;
    #[cfg(target_os = "motor")]
    match std::env::args().nth(1).as_deref() {
        Some("backing") => motor::backing(),
        Some("fault") => motor::fault(),
        Some("tls") => {
            tls::run();
            Ok(())
        }
        _ => anyhow::bail!("usage: motor-javy-tests backing|fault|tls"),
    }
    #[cfg(not(target_os = "motor"))]
    anyhow::bail!("native backing checks require Motor OS")
}
