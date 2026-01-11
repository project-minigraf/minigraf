use crate::graph::storage::GraphStorage;
use crate::query::executor::{QueryExecutor, QueryResult};
use crate::query::parser::parse_query;
use std::io::{self, Write};

pub struct Repl {
    storage: GraphStorage,
}

impl Repl {
    pub fn new(storage: GraphStorage) -> Self {
        Repl { storage }
    }

    pub fn run(&self) {
        println!("Minigraf v0.1.0 - Interactive Graph Query Console");
        println!("Type HELP for available commands, EXIT to quit.\n");

        let executor = QueryExecutor::new(&self.storage);

        loop {
            print!("minigraf> ");
            io::stdout().flush().unwrap();

            let mut input = String::new();
            match io::stdin().read_line(&mut input) {
                Ok(_) => {
                    let input = input.trim();

                    if input.is_empty() {
                        continue;
                    }

                    match parse_query(input) {
                        Ok(query) => {
                            match executor.execute(query) {
                                Ok(result) => {
                                    let formatted = result.format();
                                    println!("{}", formatted);

                                    if matches!(result, QueryResult::Exit) {
                                        break;
                                    }
                                }
                                Err(e) => {
                                    eprintln!("Execution error: {}", e);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("Parse error: {}", e);
                        }
                    }

                    println!();
                }
                Err(e) => {
                    eprintln!("Error reading input: {}", e);
                    break;
                }
            }
        }
    }
}
