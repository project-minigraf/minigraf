use minigraf::{Minigraf, OpenOptions, Repl};

fn main() -> anyhow::Result<()> {
    // Simple --file <path> argument parsing (no extra deps)
    let args: Vec<String> = std::env::args().collect();
    let db_path = args.windows(2)
        .find(|w| w[0] == "--file")
        .map(|w| w[1].clone());

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
