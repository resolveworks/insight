// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(
    all(not(debug_assertions), not(feature = "headless")),
    windows_subsystem = "windows"
)]

use clap::Parser;

#[derive(Parser, Debug)]
#[command(name = "insight")]
#[command(about = "Local-first P2P document search for journalists")]
struct Args {
    /// Run in headless mode (no GUI)
    #[arg(long)]
    headless: bool,
}

fn main() {
    let args = Args::parse();

    if args.headless {
        insight_lib::run_headless();
    } else {
        insight_lib::run();
    }
}
