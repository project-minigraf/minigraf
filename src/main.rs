use minigraf::{Minigraf, OpenOptions, Repl};

fn main() -> anyhow::Result<()> {
    // Simple --file <path> argument parsing (no extra deps)
    let args: Vec<String> = std::env::args().collect();
    let file_flag_pos = args.iter().position(|a| a == "--file");
    let db_path = file_flag_pos.and_then(|i| args.get(i + 1)).cloned();

    // Detect --file without a following value (e.g. the flag is the last argument)
    if file_flag_pos.is_some() && db_path.is_none() {
        eprintln!("error: --file requires a path argument");
        std::process::exit(1);
    }

    let db = if let Some(path) = db_path {
        OpenOptions::new().path(path).open()?
    } else {
        Minigraf::in_memory()?
    };

    let storage = db.inner_fact_storage();
    let repl = Repl::new(storage);
    repl.run();
    Ok(())
}
