use crate::graph::FactStorage;
use crate::query::datalog::{parse_datalog_command, DatalogExecutor};
use std::io::{self, Write};

pub struct Repl {
    fact_storage: FactStorage,
}

impl Repl {
    pub fn new(fact_storage: FactStorage) -> Self {
        Repl { fact_storage }
    }

    pub fn run(&self) {
        println!("Minigraf v0.1.0 - Interactive Datalog Console");
        println!("Commands: (transact [...]), (query [...]), (retract [...])");
        println!("Type EXIT to quit.\n");

        let datalog_executor = DatalogExecutor::new(self.fact_storage.clone());

        loop {
            print!("minigraf> ");
            io::stdout().flush().unwrap();

            let mut input = String::new();
            match io::stdin().read_line(&mut input) {
                Ok(n) => {
                    // EOF reached (stdin closed)
                    if n == 0 {
                        break;
                    }

                    let input = input.trim();

                    if input.is_empty() {
                        continue;
                    }

                    // Check for EXIT command
                    if input.to_uppercase() == "EXIT" {
                        break;
                    }

                    // Parse and execute Datalog command
                    match parse_datalog_command(input) {
                        Ok(command) => match datalog_executor.execute(command) {
                            Ok(result) => {
                                self.print_result(result);
                            }
                            Err(e) => {
                                eprintln!("Execution error: {}", e);
                            }
                        },
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

    fn print_result(&self, result: crate::query::datalog::QueryResult) {
        use crate::query::datalog::QueryResult as DResult;

        match result {
            DResult::Transacted(tx_id) => {
                println!("✓ Transacted successfully (tx: {})", tx_id);
            }
            DResult::Retracted(tx_id) => {
                println!("✓ Retracted successfully (tx: {})", tx_id);
            }
            DResult::QueryResults { vars, results } => {
                if results.is_empty() {
                    println!("No results found.");
                } else {
                    // Print header
                    println!("{}", vars.join("\t"));
                    println!("{}", "-".repeat(vars.len() * 20));

                    // Print rows
                    for row in &results {
                        let formatted_row: Vec<String> =
                            row.iter().map(|v| self.format_value(v)).collect();
                        println!("{}", formatted_row.join("\t"));
                    }

                    println!("\n{} result(s) found.", results.len());
                }
            }
            DResult::Ok => {
                println!("✓ OK");
            }
        }
    }

    fn format_value(&self, value: &crate::graph::types::Value) -> String {
        use crate::graph::types::Value;

        match value {
            Value::String(s) => format!("\"{}\"", s),
            Value::Integer(i) => i.to_string(),
            Value::Float(f) => f.to_string(),
            Value::Boolean(b) => b.to_string(),
            Value::Ref(uuid) => format!("#uuid {}", uuid),
            Value::Keyword(k) => k.clone(),
            Value::Null => "nil".to_string(),
        }
    }
}
