use crate::db::Minigraf;
use std::io::{self, BufRead, IsTerminal, Write};

/// An interactive REPL for a [`Minigraf`] database.
///
/// Construct via [`Minigraf::repl`] and call [`Repl::run`] to start the session.
pub struct Repl<'a> {
    db: &'a Minigraf,
}

impl<'a> Repl<'a> {
    pub(crate) fn new(db: &'a Minigraf) -> Self {
        Repl { db }
    }

    /// Start the interactive REPL loop.
    ///
    /// Reads Datalog commands from stdin line-by-line. When stdin is a TTY,
    /// a banner and prompts are printed; when piped, output is suppressed so
    /// the REPL can be driven by scripts.
    pub fn run(&self) {
        let interactive = io::stdin().is_terminal();
        self.run_impl(io::BufReader::new(io::stdin()), interactive);
    }

    fn run_impl<R: BufRead>(&self, mut reader: R, interactive: bool) {
        if interactive {
            println!(
                "Minigraf v{} - Interactive Datalog Console",
                env!("CARGO_PKG_VERSION")
            );
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
            println!(
                "  (query [:find ?x :as-of 50 :where ...])                     - state as of tx counter 50"
            );
            println!(
                "  (query [:find ?x :as-of \"2024-01-15T10:00:00Z\" :where ...]) - state as of UTC timestamp"
            );
            println!(
                "  (query [:find ?x :valid-at \"2023-06-01\" :where ...])        - facts valid on date"
            );
            println!(
                "  (query [:find ?x :valid-at :any-valid-time :where ...])     - all facts, ignoring validity"
            );
            println!();
            println!("Note: queries without :valid-at return only currently valid facts.");
            println!();
            println!("Type EXIT to quit.\n");
        }

        let mut command_buffer = String::new();
        let mut is_multiline = false;

        loop {
            if interactive {
                if is_multiline {
                    print!("       .> ");
                } else {
                    print!("minigraf> ");
                }
                io::stdout().flush().ok();
            }

            let mut input = String::new();
            match reader.read_line(&mut input) {
                Ok(n) => {
                    if n == 0 {
                        // EOF (Ctrl-D): emit a newline so the shell prompt starts
                        // on a fresh line rather than appending to the REPL prompt.
                        if interactive {
                            println!();
                        }
                        break;
                    }

                    let line = input.trim();

                    if line.is_empty() || line.starts_with('#') {
                        continue;
                    }

                    if line.to_uppercase() == "EXIT" {
                        break;
                    }

                    if !command_buffer.is_empty() {
                        command_buffer.push(' ');
                    }
                    command_buffer.push_str(line);

                    if Self::is_command_complete(&command_buffer) {
                        match self.db.execute(&command_buffer) {
                            Ok(result) => {
                                Self::print_result(result);
                            }
                            Err(e) => {
                                eprintln!("Error: {}", e);
                            }
                        }

                        command_buffer.clear();
                        is_multiline = false;
                        if interactive {
                            println!();
                        }
                    } else {
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

    fn is_command_complete(input: &str) -> bool {
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
                    #[allow(clippy::arithmetic_side_effects)]
                    {
                        depth += 1;
                    }
                }
                ')' if !in_string => {
                    #[allow(clippy::arithmetic_side_effects)]
                    {
                        depth -= 1;
                    }
                }
                _ => {}
            }
        }

        depth == 0 && input.contains('(')
    }

    fn print_result(result: crate::query::datalog::QueryResult) {
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
                    println!("{}", vars.join("\t"));
                    println!("{}", "-".repeat(vars.len().saturating_mul(20)));

                    for row in &results {
                        let formatted_row: Vec<String> =
                            row.iter().map(Self::format_value).collect();
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

    fn format_value(value: &crate::graph::types::Value) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::Minigraf;

    #[test]
    fn eof_in_interactive_mode_exits_cleanly() {
        // Exercises the `if interactive { println!(); }` branch in the Ok(0) arm.
        // An empty Cursor reaches EOF immediately; interactive=true triggers the newline path.
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_impl(std::io::Cursor::new(b""), true);
    }

    #[test]
    fn eof_in_non_interactive_mode_exits_cleanly() {
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_impl(std::io::Cursor::new(b""), false);
    }

    #[test]
    fn exit_command_terminates_loop() {
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_impl(std::io::Cursor::new(b"EXIT\n"), false);
    }

    #[test]
    fn comment_and_blank_lines_are_skipped() {
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_impl(std::io::Cursor::new(b"# comment\n\nEXIT\n"), false);
    }

    #[test]
    fn query_with_no_results_runs_without_panic() {
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_impl(
            std::io::Cursor::new(b"(query [:find ?e :where [?e :x 1]])\nEXIT\n"),
            false,
        );
    }

    #[test]
    fn transact_and_query_with_results() {
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_impl(
            std::io::Cursor::new(
                b"(transact [[:db/add \"e1\" :item/name \"Widget\"]])\n\
                  (query [:find ?n :where [_ :item/name ?n]])\n\
                  EXIT\n",
            ),
            false,
        );
    }

    #[test]
    fn retract_command_runs_without_panic() {
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_impl(
            std::io::Cursor::new(
                b"(transact [[:db/add \"e1\" :item/name \"Widget\"]])\n\
                  (retract [[:db/retract \"e1\" :item/name \"Widget\"]])\n\
                  EXIT\n",
            ),
            false,
        );
    }

    #[test]
    fn multiline_command_is_buffered_until_complete() {
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        // The first line is an incomplete command (unmatched paren); the second
        // line completes it — exercises the `is_multiline = true` branch.
        repl.run_impl(
            std::io::Cursor::new(b"(query [:find ?e\n:where [?e :x 1]])\nEXIT\n"),
            false,
        );
    }

    #[test]
    fn interactive_mode_prints_output_after_command() {
        // interactive=true exercises the `println!()` after a successful command.
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_impl(
            std::io::Cursor::new(b"(query [:find ?e :where [?e :x 1]])\nEXIT\n"),
            true,
        );
    }

    #[test]
    fn read_error_exits_loop() {
        struct ErrorReader;
        impl std::io::Read for ErrorReader {
            fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "simulated read error",
                ))
            }
        }
        impl std::io::BufRead for ErrorReader {
            fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "simulated read error",
                ))
            }
            fn consume(&mut self, _amt: usize) {}
        }

        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_impl(ErrorReader, false);
    }
}
