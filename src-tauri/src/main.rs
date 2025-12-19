// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(
    all(not(debug_assertions), not(feature = "headless")),
    windows_subsystem = "windows"
)]

use clap::{Parser, Subcommand};

#[derive(Parser, Debug)]
#[command(name = "insight")]
#[command(about = "Local-first P2P document search for journalists")]
struct Args {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Run in headless server mode (no GUI)
    Serve,

    /// Search index operations
    Index {
        #[command(subcommand)]
        action: IndexCommand,
    },
}

#[derive(Subcommand, Debug)]
enum IndexCommand {
    /// Rebuild the search index from stored documents
    Rebuild,
    /// Show status of storage vs search index
    Status,
}

fn main() {
    let args = Args::parse();

    match args.command {
        None => {
            // Default: run GUI
            insight_lib::run();
        }
        Some(Command::Serve) => {
            insight_lib::run_headless();
        }
        Some(Command::Index { action }) => match action {
            IndexCommand::Rebuild => {
                insight_lib::cli::index_rebuild();
            }
            IndexCommand::Status => {
                insight_lib::cli::index_status();
            }
        },
    }
}
