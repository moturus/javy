use std::{fs, path::PathBuf};

use anyhow::Result;
use clap::Parser;

#[derive(clap::Parser)]
#[command(about = "Initialize a Javy plugin")]
struct Args {
    #[arg(help = "Path to the uninitialized Javy plugin")]
    input: PathBuf,

    #[arg(help = "Output path for the initialized Javy plugin")]
    output: PathBuf,

    #[arg(
        long,
        help = "Produce deterministic output by using fixed clocks and seeded PRNG. Security note: both secure_random and insecure_random become non-secure."
    )]
    deterministic: bool,
}

fn main() -> Result<()> {
    javy_motor_engine::block_on(async_main())
}

async fn async_main() -> Result<()> {
    let args = Args::parse();
    javy_motor_engine::check_authority()?;
    let wasm_bytes = fs::read(&args.input)?;
    let wasm_bytes = if args.deterministic {
        javy_plugin_processing::initialize_plugin_with_determinism(&wasm_bytes).await?
    } else {
        javy_plugin_processing::initialize_plugin(&wasm_bytes).await?
    };
    fs::write(&args.output, wasm_bytes)?;
    Ok(())
}
