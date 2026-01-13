use clap::Parser;
use minigraf::{FactStorage, FileBackend, PersistentFactStorage, Repl};
use std::path::PathBuf;

/// Minigraf - A tiny, portable, bi-temporal graph database with Datalog queries
#[derive(Parser, Debug)]
#[command(name = "minigraf")]
#[command(version = "0.1.0")]
#[command(about = "Interactive Datalog REPL for graph queries", long_about = None)]
struct Args {
    /// Path to .graph file for persistent storage (optional, uses in-memory if not specified)
    #[arg(short, long, value_name = "FILE")]
    file: Option<PathBuf>,
}

fn main() {
    let args = Args::parse();

    println!("Minigraf v0.1.0 - Datalog Graph Database");

    match args.file {
        Some(path) => {
            println!("Using file-based storage: {}\n", path.display());

            // Create FileBackend
            let backend = FileBackend::open(&path).expect("Failed to open database file");

            // Create PersistentFactStorage
            let mut persistent_storage =
                PersistentFactStorage::new(backend).expect("Failed to initialize database");

            // Clone the storage for REPL
            let storage = persistent_storage.storage().clone();

            // Run REPL
            let repl = Repl::new(storage.clone());
            repl.run();

            // Mark as dirty and save on exit
            persistent_storage.mark_dirty();
            persistent_storage
                .save()
                .expect("Failed to save database on exit");
        }
        None => {
            println!("Using in-memory storage\n");

            let storage = FactStorage::new();
            let repl = Repl::new(storage);
            repl.run();
        }
    }
}
