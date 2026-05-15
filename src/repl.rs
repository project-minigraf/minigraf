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

    /// Source `init_path` silently, without starting the interactive loop.
    ///
    /// Commands in the init file are executed exactly as if they had been
    /// typed at the prompt, but without any banner or prompts. Errors are
    /// printed to stderr and execution of the init file continues. Call
    /// [`Repl::run`] afterwards to start the interactive REPL.
    pub fn run_with_init(&self, init_path: &std::path::Path) {
        match std::fs::File::open(init_path) {
            Ok(file) => {
                self.run_impl(io::BufReader::new(file), false);
            }
            Err(e) => {
                let path = init_path.display();
                eprintln!("error: could not open init file '{path}': {e}");
            }
        }
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
        // Only print the prompt once per command, not once per blank/comment line.
        let mut prompt_needed = true;

        loop {
            if interactive && !is_multiline && prompt_needed {
                print!("minigraf> ");
                io::stdout().flush().ok();
                prompt_needed = false;
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

                    if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                        continue;
                    }

                    if line.to_uppercase() == "EXIT" {
                        break;
                    }

                    if !command_buffer.is_empty() {
                        command_buffer.push('\n');
                    }
                    command_buffer.push_str(line);

                    if Self::is_command_complete(&command_buffer) {
                        match self.db.execute(&command_buffer) {
                            Ok(result) => {
                                Self::print_result(result);
                                io::stdout().flush().ok();
                            }
                            Err(e) => {
                                eprintln!("Error: {}", e);
                            }
                        }

                        command_buffer.clear();
                        is_multiline = false;
                        prompt_needed = true;
                        if interactive {
                            println!();
                            io::stdout().flush().ok();
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
    fn run_public_method_exits_on_non_tty_stdin() {
        // In TTY environments (local dev) stdin.is_terminal() is true and run() would
        // block waiting for input — skip the test gracefully.  In CI the test binary's
        // stdin is a closed pipe, so read_line() returns Ok(0) immediately and run()
        // returns after one loop iteration.  This exercises the two otherwise-uncovered
        // lines in the public `run()` wrapper (is_terminal + run_impl call).
        if io::stdin().is_terminal() {
            return;
        }
        let db = Minigraf::in_memory().expect("in-memory db");
        db.repl().run();
    }

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
        repl.run_impl(
            std::io::Cursor::new(b"# hash comment\n; edn comment\n\nEXIT\n"),
            false,
        );
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

    #[test]
    fn query_integer_result_covers_format_value_integer() {
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_impl(
            std::io::Cursor::new(
                b"(transact [[:db/add \"e1\" :item/count 42]])\n\
                  (query [:find ?c :where [_ :item/count ?c]])\n\
                  EXIT\n",
            ),
            false,
        );
    }

    #[test]
    fn query_float_result_covers_format_value_float() {
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_impl(
            std::io::Cursor::new(
                b"(transact [[:db/add \"e1\" :item/price 9.99]])\n\
                  (query [:find ?p :where [_ :item/price ?p]])\n\
                  EXIT\n",
            ),
            false,
        );
    }

    #[test]
    fn query_keyword_result_covers_format_value_keyword() {
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_impl(
            std::io::Cursor::new(
                b"(transact [[:db/add \"e1\" :item/status :active]])\n\
                  (query [:find ?s :where [_ :item/status ?s]])\n\
                  EXIT\n",
            ),
            false,
        );
    }

    #[test]
    fn query_boolean_result_covers_format_value_boolean() {
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_impl(
            std::io::Cursor::new(
                b"(transact [[:db/add \"e1\" :item/in-stock true]])\n\
                  (query [:find ?s :where [_ :item/in-stock ?s]])\n\
                  EXIT\n",
            ),
            false,
        );
    }

    #[test]
    fn rule_definition_covers_result_ok_arm() {
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_impl(
            std::io::Cursor::new(
                b"(rule [(parent ?x ?y) [?x :parent/of ?y]])\n\
                  EXIT\n",
            ),
            false,
        );
    }

    #[test]
    fn multiline_command_in_interactive_mode_covers_continuation_prompt() {
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        // First line is incomplete (unmatched paren) → no continuation prompt (suppressed in multiline mode).
        repl.run_impl(
            std::io::Cursor::new(b"(query [:find ?e\n:where [?e :x 1]])\nEXIT\n"),
            true,
        );
    }

    // --- Direct tests for print_result ---

    #[test]
    fn print_result_transacted() {
        use crate::query::datalog::QueryResult as DResult;
        Repl::print_result(DResult::Transacted(12345678));
    }

    #[test]
    fn print_result_retracted() {
        use crate::query::datalog::QueryResult as DResult;
        Repl::print_result(DResult::Retracted(12345678));
    }

    #[test]
    fn print_result_ok() {
        use crate::query::datalog::QueryResult as DResult;
        Repl::print_result(DResult::Ok);
    }

    #[test]
    fn print_result_query_no_results() {
        use crate::query::datalog::QueryResult as DResult;
        Repl::print_result(DResult::QueryResults {
            vars: vec!["?x".to_string()],
            results: vec![],
        });
    }

    #[test]
    fn print_result_query_with_rows() {
        use crate::graph::types::Value;
        use crate::query::datalog::QueryResult as DResult;
        Repl::print_result(DResult::QueryResults {
            vars: vec!["?x".to_string(), "?y".to_string()],
            results: vec![vec![Value::String("hello".to_string()), Value::Integer(42)]],
        });
    }

    // --- Direct tests for format_value ---

    #[test]
    fn format_value_string() {
        use crate::graph::types::Value;
        assert_eq!(
            Repl::format_value(&Value::String("hi".to_string())),
            "\"hi\""
        );
    }

    #[test]
    fn format_value_integer() {
        use crate::graph::types::Value;
        assert_eq!(Repl::format_value(&Value::Integer(7)), "7");
    }

    #[test]
    fn format_value_float() {
        use crate::graph::types::Value;
        assert_eq!(Repl::format_value(&Value::Float(3.14)), "3.14");
    }

    #[test]
    fn format_value_boolean() {
        use crate::graph::types::Value;
        assert_eq!(Repl::format_value(&Value::Boolean(true)), "true");
    }

    #[test]
    fn format_value_ref() {
        use crate::graph::types::Value;
        let id = uuid::Uuid::new_v4();
        assert_eq!(Repl::format_value(&Value::Ref(id)), format!("#uuid {}", id));
    }

    #[test]
    fn format_value_keyword() {
        use crate::graph::types::Value;
        assert_eq!(
            Repl::format_value(&Value::Keyword(":active".to_string())),
            ":active"
        );
    }

    #[test]
    fn format_value_null() {
        use crate::graph::types::Value;
        assert_eq!(Repl::format_value(&Value::Null), "nil");
    }

    // --- Direct tests for is_command_complete ---

    #[test]
    fn is_command_complete_handles_escaped_quote_in_string() {
        // Exercises the `escape_next` branches (lines 135-136, 140-141).
        assert!(Repl::is_command_complete(
            r#"(query [:find ?x :where [?x :name "he said \"hi\""]])"#
        ));
    }

    #[test]
    fn is_command_complete_default_arm() {
        // A plain character inside a string hits the `_ => {}` arm.
        assert!(Repl::is_command_complete(r#"(foo "bar")"#));
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn run_with_init_sources_file() {
        // run_with_init only processes the init file (no interactive loop), so
        // this test is safe to run in any environment without blocking on stdin.
        use std::io::Write;
        let mut tmp = tempfile::NamedTempFile::new().expect("tempfile");
        writeln!(
            tmp,
            "(transact [[:alice :person/name \"Alice\"]])\n\
             (rule [(has-name ?e ?n) [?e :person/name ?n]])"
        )
        .expect("write init");
        tmp.flush().expect("flush");

        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_with_init(tmp.path());
        let result = db
            .execute("(query [:find ?n :where (has-name _ ?n)])")
            .expect("query");
        match result {
            crate::query::datalog::QueryResult::QueryResults { results, .. } => {
                assert_eq!(results.len(), 1);
            }
            _ => panic!("expected query results"),
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn run_with_init_missing_file_prints_error() {
        // A non-existent init path should print an error to stderr but not panic.
        // run_with_init no longer starts the interactive loop, so no stdin blocking.
        let db = Minigraf::in_memory().expect("in-memory db");
        let repl = db.repl();
        repl.run_with_init(std::path::Path::new("/nonexistent/path/rules.datalog"));
    }
}
