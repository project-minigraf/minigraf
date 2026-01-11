use minigraf::{GraphStorage, Repl};

fn main() {
    println!("Minigraf v0.1.0 - Graph Query Language Engine");
    println!("Using in-memory storage\n");

    let storage = GraphStorage::new();
    let repl = Repl::new(storage);
    repl.run();
}
