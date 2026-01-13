use crate::graph::storage::GraphStorage;
use crate::graph::FactStorage;
use crate::query::datalog::{parse_datalog_command, DatalogExecutor};
use crate::query::executor::{QueryExecutor, QueryResult};
use crate::query::parser::parse_query;
use std::io::{self, Write};

pub struct Repl {
    graph_storage: GraphStorage,
    fact_storage: FactStorage,
}

impl Repl {
    pub fn new(graph_storage: GraphStorage) -> Self {
        Repl {
            graph_storage,
            fact_storage: FactStorage::new(),
        }
    }

    pub fn run(&self) {
        println!("Minigraf v0.1.0 - Interactive Datalog Console");
        println!("Datalog commands: (transact [...]), (query [...]), (retract [...])");
        println!("Type EXIT to quit.\n");

        let gql_executor = QueryExecutor::new(&self.graph_storage);
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

                    // Try Datalog syntax first (starts with parenthesis)
                    if input.starts_with('(') {
                        match parse_datalog_command(input) {
                            Ok(command) => match datalog_executor.execute(command) {
                                Ok(result) => {
                                    self.print_datalog_result(result);
                                }
                                Err(e) => {
                                    eprintln!("Execution error: {}", e);
                                }
                            },
                            Err(e) => {
                                eprintln!("Parse error: {}", e);
                            }
                        }
                    } else {
                        // Fall back to GQL syntax (backward compatibility)
                        match parse_query(input) {
                            Ok(query) => match gql_executor.execute(query) {
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
                            },
                            Err(e) => {
                                eprintln!("Parse error: {}", e);
                            }
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

    fn print_datalog_result(&self, result: crate::query::datalog::QueryResult) {
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
                        let formatted_row: Vec<String> = row
                            .iter()
                            .map(|v| self.format_value(v))
                            .collect();
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
