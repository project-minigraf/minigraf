//! Builds native v8 fixtures outside every measured WebAssembly instance.
use minigraf::{Minigraf, OpenOptions};
use std::{env, fs, path::Path};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let directory = env::args().nth(1).ok_or("expected output directory")?;
    fs::create_dir_all(&directory)?;
    for mib in [4_u64, 40] {
        let path = Path::new(&directory).join(format!("{mib}mib.graph"));
        if path.exists() {
            return Err("fixture already exists".into());
        }
        let db = Minigraf::open_with_options(&path, OpenOptions::new())?;
        db.execute("(transact [[:probe :answer 42]])")?;
        let mut count = 0;
        loop {
            let mut statement = String::from("(transact [");
            for i in count..count + 128 {
                statement.push_str(&format!(
                    "[:item-{i} :payload \"{i:08}{}\"] [:item-{i} :id {i}] ",
                    "x".repeat(1000)
                ));
            }
            statement.push_str("])");
            db.execute(&statement)?;
            db.checkpoint()?;
            count += 128;
            if fs::metadata(&path)?.len() >= mib * 1024 * 1024 {
                break;
            }
        }
        if !db.verify()?.is_ok() {
            return Err("fixture failed integrity verification".into());
        }
        println!(
            "{}: {} bytes, {} payload entities",
            path.display(),
            fs::metadata(&path)?.len(),
            count
        );
    }
    Ok(())
}
