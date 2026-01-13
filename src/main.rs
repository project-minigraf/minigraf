use minigraf::{FactStorage, Repl};

fn main() {
    println!("Minigraf v0.1.0 - Datalog Graph Database");
    println!("Using in-memory storage\n");

    let storage = FactStorage::new();
    let repl = Repl::new(storage);
    repl.run();
}
