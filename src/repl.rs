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
        println!("Minigraf v0.4.0 - Interactive Datalog Console");
        println!();
        println!("Data operations:");
        println!("  (transact [...])                    - assert facts");
        println!("  (transact {{:valid-from ... :valid-to ...}} [...]) - with valid time");
        println!("  (retract [...])                     - retract facts");
        println!();
        println!("Queries:");
        println!("  (query [:find ?x :where ...])       - basic query");
        println!("  (rule [(name ?a ?b) [?a :attr ?b]]) - define a rule");
        println!();
        println!("Temporal queries:");
        println!("  (query [:find ?x :as-of 50 :where ...])                     - state as of tx counter 50");
        println!("  (query [:find ?x :as-of \"2024-01-15T10:00:00Z\" :where ...]) - state as of UTC timestamp");
        println!("  (query [:find ?x :valid-at \"2023-06-01\" :where ...])        - facts valid on date");
        println!("  (query [:find ?x :valid-at :any-valid-time :where ...])     - all facts, ignoring validity");
        println!();
        println!("Note: queries without :valid-at return only currently valid facts.");
        println!();
        println!("Type EXIT to quit.\n");

        let datalog_executor = DatalogExecutor::new(self.fact_storage.clone());
        let mut command_buffer = String::new();
        let mut is_multiline = false;

        loop {
            // Show appropriate prompt
            if is_multiline {
                print!("       .> ");
            } else {
                print!("minigraf> ");
            }
            io::stdout().flush().unwrap();

            let mut input = String::new();
            match io::stdin().read_line(&mut input) {
                Ok(n) => {
                    // EOF reached (stdin closed)
                    if n == 0 {
                        break;
                    }

                    let line = input.trim();

                    // Skip empty lines and comment lines
                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }

                    // Check for EXIT command
                    if line.to_uppercase() == "EXIT" {
                        break;
                    }

                    // Accumulate input for multi-line commands
                    if !command_buffer.is_empty() {
                        command_buffer.push(' ');
                    }
                    command_buffer.push_str(line);

                    // Check if we have a complete command (balanced parentheses)
                    if self.is_command_complete(&command_buffer) {
                        // Parse and execute the complete command
                        match parse_datalog_command(&command_buffer) {
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

                        // Reset buffer
                        command_buffer.clear();
                        is_multiline = false;
                        println!();
                    } else {
                        // Command is incomplete, continue reading
                        is_multiline = true;
                    }
                }
                Err(e) => {
                    eprintln!("Error reading input: {}", e);
                    break;
                }
            }
        }
    }

    /// Check if a command has balanced parentheses (is complete)
    fn is_command_complete(&self, input: &str) -> bool {
        let mut depth = 0;
        let mut in_string = false;
        let mut escape_next = false;

        for ch in input.chars() {
            if escape_next {
                escape_next = false;
                continue;
            }

            match ch {
                '\\' if in_string => {
                    escape_next = true;
                }
                '"' => {
                    in_string = !in_string;
                }
                '(' if !in_string => {
                    depth += 1;
                }
                ')' if !in_string => {
                    depth -= 1;
                }
                _ => {}
            }
        }

        // Command is complete if we have balanced parens and at least one opening paren
        depth == 0 && input.contains('(')
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
