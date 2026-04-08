use minigraf::{Minigraf, OpenOptions};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let file_flag_pos = args.iter().position(|a| a == "--file");
    let db_path = file_flag_pos.and_then(|i| args.get(i + 1)).cloned();

    if file_flag_pos.is_some() && db_path.is_none() {
        eprintln!("error: --file requires a path argument");
        std::process::exit(1);
    }

    let db = if let Some(path) = db_path {
        OpenOptions::new().path(path).open()?
    } else {
        Minigraf::in_memory()?
    };

    db.repl().run();
    Ok(())
}
