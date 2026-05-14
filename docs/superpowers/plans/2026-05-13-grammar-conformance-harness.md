# Grammar Specification and Conformance Harness — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build a `pest` shadow grammar, a three-bucket `.edn` corpus, a conformance test harness that keeps them in sync with the real parser, and publish a formal EBNF grammar + semantics document to the wiki.

**Architecture:** A `pest` PEG grammar (`tests/grammar/grammar.pest`) encodes the structural syntax. Three corpus buckets (`valid/`, `invalid/syntax/`, `invalid/semantic/`) define expected parse outcomes. `tests/grammar_conformance.rs` runs both parsers over every file and asserts the correct contract per bucket. The EBNF documentation in `.wiki/Datalog-Reference.md` is derived from the `pest` grammar.

**Tech Stack:** Rust, `pest` 2.x + `pest_derive` 2.x (dev-only), `minigraf::Minigraf` public API.

**Spec:** `docs/superpowers/specs/2026-05-08-grammar-conformance-design.md`

**Correction from spec:** `not_join_insufficient_args.edn` turns out to be a syntax case (pest rejects `(not-join [?v])` because `where_clause+` requires ≥1 clause). Replaced in the semantic bucket with `not_join_unbound_join_var.edn`. The extra syntax case is `not_join_no_clauses.edn`. The spec's semantic table entry for `fact_too_few_elements.edn` and `retract_wrong_arity.edn` is preserved by using a permissive `fact = { "[" ~ edn_value+ ~ "]" }` rule so pest accepts under-/over-arity facts.

---

## File Map

| Action | Path | Purpose |
|--------|------|---------|
| Modify | `Cargo.toml` | Add `pest`, `pest_derive` to `[dev-dependencies]` |
| Create | `tests/grammar/grammar.pest` | Pest shadow grammar — grows across Tasks 2–8 |
| Create | `tests/grammar/valid/*.edn` | 31 valid corpus fixtures |
| Create | `tests/grammar/invalid/syntax/*.edn` | 10 syntax-error fixtures |
| Create | `tests/grammar/invalid/semantic/*.edn` | 13 semantic-error fixtures |
| Create | `tests/grammar_conformance.rs` | Three `#[test]` functions, one per bucket |
| Modify | `.wiki/Datalog-Reference.md` | Add EBNF grammar + semantics sections |

---

## Task 1: Cargo dependencies, directory skeleton, and conformance test harness

**Files:**
- Modify: `Cargo.toml`
- Create: `tests/grammar/grammar.pest`
- Create: `tests/grammar_conformance.rs`

- [ ] **Step 1: Add `pest` and `pest_derive` to dev-dependencies in `Cargo.toml`**

  In `Cargo.toml`, find the `[dev-dependencies]` section and add:

  ```toml
  [dev-dependencies]
  serde_json = "1.0"
  pest = "2"
  pest_derive = "2"
  ```

- [ ] **Step 2: Create the corpus directory structure**

  ```bash
  mkdir -p tests/grammar/valid
  mkdir -p tests/grammar/invalid/syntax
  mkdir -p tests/grammar/invalid/semantic
  ```

- [ ] **Step 3: Create a minimal placeholder `grammar.pest`**

  Create `tests/grammar/grammar.pest` with a rule that rejects everything:

  ```pest
  // Minigraf Datalog grammar — built incrementally; see implementation plan.
  // This placeholder rejects all input until proper rules are added in Task 2.
  command = { SOI ~ EOI }
  ```

- [ ] **Step 4: Write `tests/grammar_conformance.rs`**

  ```rust
  //! Grammar conformance tests.
  //!
  //! Three test functions verify that `grammar.pest` and the real parser agree:
  //!
  //! - `valid_corpus`:           pest ACCEPTS  + parser ACCEPTS
  //! - `invalid_syntax_corpus`:  pest REJECTS  + parser REJECTS
  //! - `invalid_semantic_corpus`:pest ACCEPTS  + parser REJECTS
  //!
  //! Run with: cargo test grammar_conformance -- --nocapture

  use pest::Parser;
  use pest_derive::Parser;
  use std::fs;
  use std::path::Path;

  #[derive(Parser)]
  #[grammar = "tests/grammar/grammar.pest"]
  struct DatalogGrammar;

  /// Returns true when the pest grammar accepts the full input as a `command`.
  fn pest_accepts(input: &str) -> bool {
      DatalogGrammar::parse(Rule::command, input.trim()).is_ok()
  }

  /// Returns true when the real Minigraf parser accepts the input.
  ///
  /// Uses `db.prepare()` for `(query ...)` commands so that bind-slot
  /// templates parse correctly without needing substituted values at test
  /// time.  Uses `db.execute()` for transact / retract / rule.
  fn parser_accepts(input: &str) -> bool {
      let input = input.trim();
      let db = minigraf::Minigraf::in_memory().expect("in-memory db");
      if input.starts_with("(query") {
          db.prepare(input).is_ok()
      } else {
          db.execute(input).is_ok()
      }
  }

  /// Read every `.edn` file in `dir`, returning `(filename, content)` pairs
  /// sorted by filename for deterministic test output.
  fn load_corpus(dir: &str) -> Vec<(String, String)> {
      let path = Path::new(dir);
      if !path.exists() {
          return vec![];
      }
      let mut files: Vec<(String, String)> = fs::read_dir(path)
          .unwrap_or_else(|_| panic!("cannot read dir {dir}"))
          .filter_map(|e| {
              let e = e.ok()?;
              let p = e.path();
              if p.extension()?.to_str()? == "edn" {
                  let name = p.file_name()?.to_str()?.to_string();
                  let content = fs::read_to_string(&p).ok()?;
                  Some((name, content))
              } else {
                  None
              }
          })
          .collect();
      files.sort_by(|a, b| a.0.cmp(&b.0));
      files
  }

  // ── VALID ─────────────────────────────────────────────────────────────────

  #[test]
  fn valid_corpus() {
      let files = load_corpus("tests/grammar/valid");
      assert!(!files.is_empty(), "valid/ corpus is empty — add .edn fixtures");
      let mut failures: Vec<String> = vec![];
      for (name, content) in &files {
          if !pest_accepts(content) {
              failures.push(format!("FAIL valid/{name}: pest rejected (expected accept)"));
          }
          if !parser_accepts(content) {
              failures.push(format!("FAIL valid/{name}: parser rejected (expected accept)"));
          }
      }
      if !failures.is_empty() {
          panic!("\n{}", failures.join("\n"));
      }
  }

  // ── INVALID SYNTAX ────────────────────────────────────────────────────────

  #[test]
  fn invalid_syntax_corpus() {
      let files = load_corpus("tests/grammar/invalid/syntax");
      assert!(!files.is_empty(), "invalid/syntax/ corpus is empty — add .edn fixtures");
      let mut failures: Vec<String> = vec![];
      for (name, content) in &files {
          if pest_accepts(content) {
              failures.push(format!(
                  "FAIL invalid/syntax/{name}: pest accepted (expected reject — move to valid/ or tighten grammar)"
              ));
          }
          if parser_accepts(content) {
              failures.push(format!(
                  "FAIL invalid/syntax/{name}: parser accepted (expected reject)"
              ));
          }
      }
      if !failures.is_empty() {
          panic!("\n{}", failures.join("\n"));
      }
  }

  // ── INVALID SEMANTIC ──────────────────────────────────────────────────────

  #[test]
  fn invalid_semantic_corpus() {
      let files = load_corpus("tests/grammar/invalid/semantic");
      assert!(!files.is_empty(), "invalid/semantic/ corpus is empty — add .edn fixtures");
      let mut failures: Vec<String> = vec![];
      for (name, content) in &files {
          if !pest_accepts(content) {
              failures.push(format!(
                  "FAIL invalid/semantic/{name}: pest rejected (expected accept — move to invalid/syntax/ or loosen grammar)"
              ));
          }
          if parser_accepts(content) {
              failures.push(format!(
                  "FAIL invalid/semantic/{name}: parser accepted (expected reject — move to valid/ or tighten semantic check)"
              ));
          }
      }
      if !failures.is_empty() {
          panic!("\n{}", failures.join("\n"));
      }
  }
  ```

- [ ] **Step 5: Verify the harness compiles and the three tests pass (empty dirs → trivially pass)**

  ```bash
  cargo test grammar_conformance -- --nocapture
  ```

  Expected:
  ```
  test valid_corpus ... FAILED   (empty corpus assertion — expected)
  ```

  Wait — the empty-dir assertions will fail. Temporarily comment them out to verify compilation:

  ```bash
  cargo test grammar_conformance -- --nocapture 2>&1 | grep -E "^(test |error)"
  ```

  Expected: all three tests either pass or fail with "corpus is empty" — no compilation error.

- [ ] **Step 6: Commit**

  ```bash
  git add Cargo.toml tests/grammar/grammar.pest tests/grammar_conformance.rs
  git commit -m "test: add grammar conformance harness skeleton (Task 1)"
  ```

---

## Task 2: EDN primitives + transact + retract

**Files:**
- Modify: `tests/grammar/grammar.pest` (replace placeholder with full primitives + commands)
- Create: `tests/grammar/valid/transact_basic.edn`
- Create: `tests/grammar/valid/transact_valid_time_tx_level.edn`
- Create: `tests/grammar/valid/transact_valid_time_per_fact.edn`
- Create: `tests/grammar/valid/transact_valid_time_both.edn`
- Create: `tests/grammar/valid/retract_basic.edn`
- Create: `tests/grammar/valid/edn_all_value_types.edn`
- Create: `tests/grammar/valid/edn_uuid.edn`
- Create: `tests/grammar/valid/edn_string_escapes.edn`

- [ ] **Step 1: Write the corpus files**

  `tests/grammar/valid/transact_basic.edn`:
  ```edn
  (transact [[:alice :person/name "Alice"]
             [:bob :person/name "Bob"]])
  ```

  `tests/grammar/valid/transact_valid_time_tx_level.edn`:
  ```edn
  (transact {:valid-from "2024-01-01T00:00:00Z" :valid-to "2025-01-01T00:00:00Z"}
            [[:alice :person/name "Alice"]])
  ```

  `tests/grammar/valid/transact_valid_time_per_fact.edn`:
  ```edn
  (transact [[:alice :contract/active true {:valid-from "2024-01-01T00:00:00Z" :valid-to "2025-01-01T00:00:00Z"}]])
  ```

  `tests/grammar/valid/transact_valid_time_both.edn`:
  ```edn
  (transact {:valid-from "2024-01-01T00:00:00Z"}
            [[:alice :contract/active true {:valid-to "2025-01-01T00:00:00Z"}]])
  ```

  `tests/grammar/valid/retract_basic.edn`:
  ```edn
  (retract [[:alice :person/name "Alice"]])
  ```

  `tests/grammar/valid/edn_all_value_types.edn`:
  ```edn
  (transact [[:e1 :type/string "hello world"]
             [:e2 :type/integer 42]
             [:e3 :type/neg-int -7]
             [:e4 :type/float 3.14]
             [:e5 :type/neg-float -2.5]
             [:e6 :type/bool-true true]
             [:e7 :type/bool-false false]
             [:e8 :type/nil nil]
             [:e9 :type/ref :other-entity]])
  ```

  `tests/grammar/valid/edn_uuid.edn`:
  ```edn
  (transact [[#uuid "550e8400-e29b-41d4-a716-446655440000" :entity/name "by-uuid"]])
  ```

  `tests/grammar/valid/edn_string_escapes.edn`:
  ```edn
  (transact [[:e :str/newline "line1\nline2"]
             [:e :str/tab "col1\tcol2"]
             [:e :str/quote "say \"hello\""]
             [:e :str/backslash "path\\to\\file"]])
  ```

- [ ] **Step 2: Run the conformance test — expect failures because the grammar still rejects everything**

  ```bash
  cargo test grammar_conformance::valid_corpus -- --nocapture
  ```

  Expected: FAIL — "pest rejected" for all 8 files.

- [ ] **Step 3: Replace `tests/grammar/grammar.pest` with the primitives + transact/retract grammar**

  Replace the entire contents of `tests/grammar/grammar.pest` with:

  ```pest
  // ── WHITESPACE (auto-skipped between every token) ─────────────────────────
  WHITESPACE = _{ " " | "\t" | "\n" | "\r" | "," }

  // ── WORD-BOUNDARY HELPER ──────────────────────────────────────────────────
  // Used in negative lookaheads to enforce keyword boundaries.
  word_cont = _{ ASCII_ALPHANUMERIC | "?" | "_" | "-" | "/" }

  // ── EDN PRIMITIVES ────────────────────────────────────────────────────────

  // boolean: "true" or "false" — only when NOT followed by a word character
  // (so "trueish" is NOT a boolean, it's a plain symbol)
  boolean_lit = @{ ("true" | "false") ~ !word_cont }

  // nil literal
  nil_lit = @{ "nil" ~ !word_cont }

  // integer: optional minus, one or more digits, NOT followed by "." + digit
  // (the negative lookahead prevents consuming "-3" as the start of "-3.14")
  integer_lit = @{ "-"? ~ ASCII_DIGIT+ ~ !("." ~ ASCII_DIGIT) }

  // float: optional minus, digits, dot, optional more digits
  float_lit = @{ "-"? ~ ASCII_DIGIT+ ~ "." ~ ASCII_DIGIT* }

  // string with escape sequences: \n \t \r \" \\, or any other char after \
  str_char  = _{ "\\" ~ ("n" | "t" | "r" | "\"" | "\\" | ANY) | !("\"" | "\\") ~ ANY }
  string_lit = @{ "\"" ~ str_char* ~ "\"" }

  // UUID tagged literal: #uuid "xxxxxxxx-xxxx-xxxx-xxxx-xxxxxxxxxxxx"
  // Content validation (RFC 4122 format) is a semantic check — pest accepts any string.
  uuid_lit = { "#uuid" ~ string_lit }

  // Prepared-query bind slot: $name
  bind_slot = @{ "$" ~ (ASCII_ALPHANUMERIC | "_" | "-")+ }

  // Keyword: starts with ":", followed by alphanumeric, "/", "-", "_"
  keyword = @{ ":" ~ (ASCII_ALPHANUMERIC | "/" | "-" | "_")+ }

  // Logic variable: starts with "?"
  variable = @{ "?" ~ word_cont* }

  // Operator symbols used in expressions
  op_sym = @{ "<=" | ">=" | "!=" | "<" | ">" | "=" | "+" | "*" | "/" }

  // Plain symbol: letter or underscore start; or "-" followed by at least one word char.
  // Negative lookaheads prevent "true", "false", "nil" from matching as plain symbols.
  plain_sym = @{
      !boolean_lit ~ !nil_lit ~ (ASCII_ALPHA | "_") ~ word_cont*
      | "-" ~ word_cont+
  }

  // ── EDN VALUE (any EDN atom or container) ─────────────────────────────────
  // Order matters: more specific alternatives first to avoid ambiguity.
  edn_value = {
      uuid_lit | boolean_lit | nil_lit | float_lit | integer_lit | string_lit |
      bind_slot | keyword | op_sym | variable | plain_sym |
      edn_list | edn_vector | edn_map
  }
  edn_list   = { "(" ~ edn_value* ~ ")" }
  edn_vector = { "[" ~ edn_value* ~ "]" }
  edn_map    = { "{" ~ (edn_value ~ edn_value)* ~ "}" }

  // ── TOP-LEVEL COMMAND ─────────────────────────────────────────────────────
  command = { SOI ~ (transact_cmd | retract_cmd | query_cmd | rule_cmd) ~ EOI }

  // ── TRANSACT ──────────────────────────────────────────────────────────────
  // (transact [facts...]) or (transact {:valid-from "ts"} [facts...])
  transact_cmd = {
      "(" ~ "transact" ~ (valid_time_map ~ fact_vector | fact_vector) ~ ")"
  }

  // ── RETRACT ───────────────────────────────────────────────────────────────
  retract_cmd = { "(" ~ "retract" ~ fact_vector ~ ")" }

  // Fact vector: a vector of individual fact vectors
  fact_vector = { "[" ~ fact* ~ "]" }

  // A single fact: permissive (edn_value+) so arity violations are semantic, not syntax.
  // The real parser enforces [E A V] or [E A V {map}]; pest only requires ≥1 element.
  fact = { "[" ~ edn_value+ ~ "]" }

  // Transaction-level valid-time options map
  valid_time_map  = { "{" ~ valid_time_pair* ~ "}" }
  valid_time_pair = { (":valid-from" | ":valid-to") ~ string_lit }

  // ── QUERY (stub — filled in Task 3+) ─────────────────────────────────────
  query_cmd = { "(" ~ "query" ~ edn_value+ ~ ")" }

  // ── RULE (stub — filled in Task 8) ───────────────────────────────────────
  rule_cmd = { "(" ~ "rule" ~ edn_value+ ~ ")" }
  ```

- [ ] **Step 4: Run conformance test — expect all 8 valid files to pass**

  ```bash
  cargo test grammar_conformance::valid_corpus -- --nocapture
  ```

  Expected: all 8 files pass (pest accepts + parser accepts).

- [ ] **Step 5: Commit**

  ```bash
  git add tests/grammar/grammar.pest tests/grammar/valid/
  git commit -m "test: add EDN primitives, transact, retract corpus + grammar (Task 2)"
  ```

---

## Task 3: Basic query — `:find` variables and `:where` patterns

**Files:**
- Create: `tests/grammar/valid/query_basic.edn`
- Modify: `tests/grammar/grammar.pest` (replace `query_cmd` stub with real rules)

- [ ] **Step 1: Write the corpus file**

  `tests/grammar/valid/query_basic.edn`:
  ```edn
  (query [:find ?name ?age
          :where [?e :person/name ?name]
                 [?e :person/age ?age]])
  ```

- [ ] **Step 2: Run — expect failure (query_cmd stub accepts any edn_value+ so this should actually pass — but verify)**

  ```bash
  cargo test grammar_conformance::valid_corpus -- --nocapture
  ```

  If it passes already (stub is permissive enough), continue to Step 3 anyway.

- [ ] **Step 3: Replace the `query_cmd` stub with the real query grammar**

  In `tests/grammar/grammar.pest`, replace:
  ```pest
  // ── QUERY (stub — filled in Task 3+) ─────────────────────────────────────
  query_cmd = { "(" ~ "query" ~ edn_value+ ~ ")" }
  ```

  With:
  ```pest
  // ── QUERY ─────────────────────────────────────────────────────────────────
  query_cmd    = { "(" ~ "query" ~ query_vector ~ ")" }
  query_vector = { "[" ~ query_section+ ~ "]" }

  // A query vector is a sequence of keyed sections in any order.
  query_section = {
      find_section | where_section | as_of_section |
      valid_at_section | any_valid_time_section | with_section
  }

  // ── FIND SECTION ──────────────────────────────────────────────────────────
  // :find accepts variables, aggregates, and window functions (Tasks 4–5 add the latter two)
  find_section = { ":find" ~ find_spec+ }
  find_spec    = { window_expr | aggregate_expr | variable }

  // Aggregate: (func-name ?var) — e.g. (count ?e), (sum ?salary)
  aggregate_expr = { "(" ~ (op_sym | plain_sym) ~ variable ~ ")" }

  // Window function: (func ?var :over (...)) or (func :over (...)) for rank/row-number
  window_expr = {
      "(" ~ (op_sym | plain_sym) ~ (variable ~ ":over" | ":over") ~ over_clause ~ ")"
  }
  over_clause = { "(" ~ over_option* ~ ")" }
  over_option = {
      ":partition-by" ~ variable |
      ":order-by" ~ variable |
      ":desc" | ":asc"
  }

  // ── WHERE SECTION ─────────────────────────────────────────────────────────
  where_section = { ":where" ~ where_clause+ }

  // where_clause covers all clause forms; expanded in Tasks 6–7.
  where_clause = {
      expr_clause | not_clause | not_join_clause |
      or_clause | or_join_clause |
      pattern_clause | rule_invocation
  }

  // Pattern: exactly [E A V] — three EDN values
  pattern_clause = { "[" ~ edn_value ~ edn_value ~ edn_value ~ "]" }

  // Expression clause: [(expr) ?out?] — first element is a list (the expression)
  expr_clause = { "[" ~ expr ~ variable? ~ "]" }

  // NOT / NOT-JOIN (stubs — content added in Task 6)
  not_clause      = { "(" ~ "not" ~ where_clause+ ~ ")" }
  not_join_clause = { "(" ~ "not-join" ~ join_vars ~ where_clause+ ~ ")" }

  // OR / OR-JOIN (stubs — content added in Task 6)
  or_clause      = { "(" ~ "or" ~ or_branch+ ~ ")" }
  or_join_clause = { "(" ~ "or-join" ~ join_vars ~ or_branch+ ~ ")" }
  join_vars      = { "[" ~ variable* ~ "]" }
  or_branch      = { and_branch | where_clause }
  and_branch     = { "(" ~ "and" ~ where_clause+ ~ ")" }

  // Rule invocation: (predicate-name args...)
  rule_invocation = { "(" ~ plain_sym ~ edn_value* ~ ")" }

  // ── EXPRESSIONS (stub — fully populated in Task 7) ────────────────────────
  expr        = { "(" ~ expr_body ~ ")" }
  expr_body   = { unary_form | binary_form }
  unary_form  = { (unary_op | op_sym | plain_sym) ~ expr_arg }
  binary_form = { binary_op ~ expr_arg ~ expr_arg }
  unary_op    = @{ "string?" | "integer?" | "float?" | "boolean?" | "nil?" }
  binary_op   = @{
      "<=" | ">=" | "!=" | "starts-with?" | "ends-with?" | "contains?" | "matches?" |
      "<" | ">" | "=" | "+" | "-" | "*" | "/"
  }
  expr_arg = {
      expr | boolean_lit | nil_lit | float_lit | integer_lit |
      string_lit | bind_slot | keyword | variable
  }

  // ── TEMPORAL / WITH SECTIONS (stubs — corpus added in Task 4) ─────────────
  as_of_section          = { ":as-of" ~ (integer_lit | string_lit | bind_slot) }
  valid_at_section       = { ":valid-at" ~ (string_lit | ":any-valid-time" | bind_slot) }
  any_valid_time_section = { ":any-valid-time" }
  with_section           = { ":with" ~ variable+ }
  ```

- [ ] **Step 4: Run — all valid corpus files must pass**

  ```bash
  cargo test grammar_conformance::valid_corpus -- --nocapture
  ```

  Expected: all 9 files pass (8 from Task 2 + `query_basic.edn`).

- [ ] **Step 5: Commit**

  ```bash
  git add tests/grammar/grammar.pest tests/grammar/valid/query_basic.edn
  git commit -m "test: add basic query corpus + full query grammar structure (Task 3)"
  ```

---

## Task 4: Temporal modifiers — `:as-of`, `:valid-at`, `:any-valid-time`

**Files:**
- Create: `tests/grammar/valid/query_as_of_counter.edn`
- Create: `tests/grammar/valid/query_as_of_timestamp.edn`
- Create: `tests/grammar/valid/query_valid_at_timestamp.edn`
- Create: `tests/grammar/valid/query_valid_at_any_valid_time.edn`
- Create: `tests/grammar/valid/query_any_valid_time_shorthand.edn`

The grammar rules for these sections were already added as stubs in Task 3. This task just adds the corpus and verifies they work.

- [ ] **Step 1: Write the corpus files**

  `tests/grammar/valid/query_as_of_counter.edn`:
  ```edn
  (query [:find ?name
          :as-of 3
          :where [?e :person/name ?name]])
  ```

  `tests/grammar/valid/query_as_of_timestamp.edn`:
  ```edn
  (query [:find ?name
          :as-of "2024-01-15T10:00:00Z"
          :where [?e :person/name ?name]])
  ```

  `tests/grammar/valid/query_valid_at_timestamp.edn`:
  ```edn
  (query [:find ?name
          :valid-at "2023-06-01T00:00:00Z"
          :where [?e :person/name ?name]])
  ```

  `tests/grammar/valid/query_valid_at_any_valid_time.edn`:
  ```edn
  (query [:find ?name
          :valid-at :any-valid-time
          :where [?e :person/name ?name]])
  ```

  `tests/grammar/valid/query_any_valid_time_shorthand.edn`:
  ```edn
  (query [:find ?name
          :any-valid-time
          :where [?e :person/name ?name]])
  ```

- [ ] **Step 2: Run — all 14 valid files must pass**

  ```bash
  cargo test grammar_conformance::valid_corpus -- --nocapture
  ```

  Expected: all 14 pass.

- [ ] **Step 3: Commit**

  ```bash
  git add tests/grammar/valid/query_as_of_counter.edn \
          tests/grammar/valid/query_as_of_timestamp.edn \
          tests/grammar/valid/query_valid_at_timestamp.edn \
          tests/grammar/valid/query_valid_at_any_valid_time.edn \
          tests/grammar/valid/query_any_valid_time_shorthand.edn
  git commit -m "test: add temporal modifier corpus (Task 4)"
  ```

---

## Task 5: Aggregates, window functions, and `:with`

**Files:**
- Create: `tests/grammar/valid/query_aggregate_count.edn`
- Create: `tests/grammar/valid/query_aggregate_sum.edn`
- Create: `tests/grammar/valid/query_aggregate_udf.edn`
- Create: `tests/grammar/valid/query_window_sum.edn`
- Create: `tests/grammar/valid/query_window_rank.edn`
- Create: `tests/grammar/valid/query_window_partition_by.edn`
- Create: `tests/grammar/valid/query_with_clause.edn`

The `find_spec`, `aggregate_expr`, `window_expr`, and `with_section` grammar rules were added in Task 3. This task adds the corpus and verifies.

- [ ] **Step 1: Write the corpus files**

  `tests/grammar/valid/query_aggregate_count.edn`:
  ```edn
  (query [:find (count ?e)
          :where [?e :person/name ?name]])
  ```

  `tests/grammar/valid/query_aggregate_sum.edn`:
  ```edn
  (query [:find (sum ?salary)
          :where [?e :emp/salary ?salary]])
  ```

  `tests/grammar/valid/query_aggregate_udf.edn`:
  ```edn
  (query [:find (geomean ?v)
          :where [?e :metric/value ?v]])
  ```

  `tests/grammar/valid/query_window_sum.edn`:
  ```edn
  (query [:find (sum ?salary :over (:order-by ?hire-date))
          :where [?e :emp/salary ?salary]
                 [?e :emp/hire-date ?hire-date]])
  ```

  `tests/grammar/valid/query_window_rank.edn`:
  ```edn
  (query [:find (rank :over (:order-by ?salary :desc))
          :where [?e :emp/salary ?salary]])
  ```

  `tests/grammar/valid/query_window_partition_by.edn`:
  ```edn
  (query [:find (sum ?salary :over (:partition-by ?dept :order-by ?hire-date))
          :where [?e :emp/salary ?salary]
                 [?e :emp/dept ?dept]
                 [?e :emp/hire-date ?hire-date]])
  ```

  `tests/grammar/valid/query_with_clause.edn`:
  ```edn
  (query [:find ?dept (sum ?salary)
          :with ?e
          :where [?e :emp/dept ?dept]
                 [?e :emp/salary ?salary]])
  ```

- [ ] **Step 2: Run — all 21 valid files must pass**

  ```bash
  cargo test grammar_conformance::valid_corpus -- --nocapture
  ```

  Expected: all 21 pass. If any aggregate/window file fails, inspect the grammar rules for `aggregate_expr` / `window_expr` added in Task 3 and fix as needed.

- [ ] **Step 3: Commit**

  ```bash
  git add tests/grammar/valid/query_aggregate_count.edn \
          tests/grammar/valid/query_aggregate_sum.edn \
          tests/grammar/valid/query_aggregate_udf.edn \
          tests/grammar/valid/query_window_sum.edn \
          tests/grammar/valid/query_window_rank.edn \
          tests/grammar/valid/query_window_partition_by.edn \
          tests/grammar/valid/query_with_clause.edn
  git commit -m "test: add aggregate, window function, and :with corpus (Task 5)"
  ```

---

## Task 6: `not`, `not-join`, `or`, `or-join`

**Files:**
- Create: `tests/grammar/valid/query_not.edn`
- Create: `tests/grammar/valid/query_not_join.edn`
- Create: `tests/grammar/valid/query_or.edn`
- Create: `tests/grammar/valid/query_or_join.edn`

The grammar rules were added as stubs in Task 3. This task adds the corpus and verifies.

- [ ] **Step 1: Write the corpus files**

  `tests/grammar/valid/query_not.edn`:
  ```edn
  (query [:find ?name
          :where [?e :person/name ?name]
                 (not [?e :person/banned true])])
  ```

  `tests/grammar/valid/query_not_join.edn`:
  ```edn
  (query [:find ?name
          :where [?e :person/name ?name]
                 (not-join [?e]
                   [?e :person/banned true])])
  ```

  `tests/grammar/valid/query_or.edn`:
  ```edn
  (query [:find ?name
          :where [?e :person/name ?name]
                 (or [?e :person/city "London"]
                     [?e :person/city "Paris"])])
  ```

  `tests/grammar/valid/query_or_join.edn`:
  ```edn
  (query [:find ?name
          :where [?e :person/name ?name]
                 (or-join [?e]
                   [?e :person/role :admin]
                   [?e :person/role :superuser])])
  ```

- [ ] **Step 2: Run — all 25 valid files must pass**

  ```bash
  cargo test grammar_conformance::valid_corpus -- --nocapture
  ```

  Expected: all 25 pass.

- [ ] **Step 3: Commit**

  ```bash
  git add tests/grammar/valid/query_not.edn \
          tests/grammar/valid/query_not_join.edn \
          tests/grammar/valid/query_or.edn \
          tests/grammar/valid/query_or_join.edn
  git commit -m "test: add not/not-join/or/or-join corpus (Task 6)"
  ```

---

## Task 7: Expression clauses

**Files:**
- Create: `tests/grammar/valid/query_expr_filter.edn`
- Create: `tests/grammar/valid/query_expr_binding.edn`
- Create: `tests/grammar/valid/query_expr_nested.edn`

The `expr_clause`, `expr`, `unary_form`, `binary_form` grammar rules were added in Task 3.

- [ ] **Step 1: Write the corpus files**

  `tests/grammar/valid/query_expr_filter.edn`:
  ```edn
  (query [:find ?name ?age
          :where [?e :person/name ?name]
                 [?e :person/age ?age]
                 [(> ?age 25)]])
  ```

  `tests/grammar/valid/query_expr_binding.edn`:
  ```edn
  (query [:find ?name ?total
          :where [?e :person/name ?name]
                 [?e :order/price ?price]
                 [?e :order/qty ?qty]
                 [(* ?price ?qty) ?total]])
  ```

  `tests/grammar/valid/query_expr_nested.edn`:
  ```edn
  (query [:find ?name ?result
          :where [?e :person/name ?name]
                 [?e :metric/a ?a]
                 [?e :metric/b ?b]
                 [(+ (* ?a 2) ?b) ?result]])
  ```

- [ ] **Step 2: Run — all 28 valid files must pass**

  ```bash
  cargo test grammar_conformance::valid_corpus -- --nocapture
  ```

  Expected: all 28 pass.

- [ ] **Step 3: Commit**

  ```bash
  git add tests/grammar/valid/query_expr_filter.edn \
          tests/grammar/valid/query_expr_binding.edn \
          tests/grammar/valid/query_expr_nested.edn
  git commit -m "test: add expression clause corpus (Task 7)"
  ```

---

## Task 8: Bind slots and rule command

**Files:**
- Create: `tests/grammar/valid/query_prepared_bind_slot.edn`
- Create: `tests/grammar/valid/rule_basic.edn`
- Create: `tests/grammar/valid/rule_recursive.edn`
- Modify: `tests/grammar/grammar.pest` (replace `rule_cmd` stub with real rule grammar)

- [ ] **Step 1: Write the corpus files**

  `tests/grammar/valid/query_prepared_bind_slot.edn`:
  ```edn
  (query [:find ?name
          :where [?e :person/name ?name]
                 [?e :person/city $city]])
  ```

  `tests/grammar/valid/rule_basic.edn`:
  ```edn
  (rule [(reachable ?from ?to)
         [?from :connected ?to]])
  ```

  `tests/grammar/valid/rule_recursive.edn`:
  ```edn
  (rule [(reachable ?from ?to)
         [?from :connected ?mid]
         (reachable ?mid ?to)])
  ```

- [ ] **Step 2: Run — expect failures for rule files (stub accepts any edn_value+ so they may already pass — verify)**

  ```bash
  cargo test grammar_conformance::valid_corpus -- --nocapture
  ```

  Note: the `rule_cmd` stub `"(" ~ "rule" ~ edn_value+ ~ ")"` may already accept the rule files. Regardless, proceed to Step 3 to replace the stub with the correct grammar.

- [ ] **Step 3: In `tests/grammar/grammar.pest`, replace the `rule_cmd` stub**

  Replace:
  ```pest
  // ── RULE (stub — filled in Task 8) ───────────────────────────────────────
  rule_cmd = { "(" ~ "rule" ~ edn_value+ ~ ")" }
  ```

  With:
  ```pest
  // ── RULE ──────────────────────────────────────────────────────────────────
  // (rule [(head ?args) body-clause ...])
  rule_cmd    = { "(" ~ "rule" ~ rule_vector ~ ")" }
  rule_vector = { "[" ~ rule_head ~ where_clause* ~ "]" }
  rule_head   = { "(" ~ plain_sym ~ variable* ~ ")" }
  ```

- [ ] **Step 4: Run — all 31 valid files must pass**

  ```bash
  cargo test grammar_conformance::valid_corpus -- --nocapture
  ```

  Expected: all 31 pass.

- [ ] **Step 5: Commit**

  ```bash
  git add tests/grammar/grammar.pest \
          tests/grammar/valid/query_prepared_bind_slot.edn \
          tests/grammar/valid/rule_basic.edn \
          tests/grammar/valid/rule_recursive.edn
  git commit -m "test: add bind-slot + rule corpus and real rule grammar (Task 8)"
  ```

---

## Task 9: `invalid/syntax/` corpus

**Files:**
- Create: `tests/grammar/invalid/syntax/unclosed_paren.edn`
- Create: `tests/grammar/invalid/syntax/unclosed_bracket.edn`
- Create: `tests/grammar/invalid/syntax/unknown_command.edn`
- Create: `tests/grammar/invalid/syntax/empty_command.edn`
- Create: `tests/grammar/invalid/syntax/unknown_tagged_literal.edn`
- Create: `tests/grammar/invalid/syntax/string_unterminated.edn`
- Create: `tests/grammar/invalid/syntax/bind_slot_empty.edn`
- Create: `tests/grammar/invalid/syntax/keyword_invalid_chars.edn`
- Create: `tests/grammar/invalid/syntax/unexpected_bare_char.edn`
- Create: `tests/grammar/invalid/syntax/not_join_no_clauses.edn`

- [ ] **Step 1: Write all 10 syntax-error corpus files**

  `tests/grammar/invalid/syntax/unclosed_paren.edn`:
  ```edn
  (transact [[:alice :person/name "Alice"]
  ```
  *(file ends with an open bracket — no closing `)`)*

  `tests/grammar/invalid/syntax/unclosed_bracket.edn`:
  ```edn
  (transact [[:alice :person/name "Alice"
  ```
  *(neither `]` nor `)` — double-unclosed)*

  `tests/grammar/invalid/syntax/unknown_command.edn`:
  ```edn
  (insert [[:alice :person/name "Alice"]])
  ```
  *(pest: `command` requires one of the four known command names)*

  `tests/grammar/invalid/syntax/empty_command.edn`:
  ```edn
  ()
  ```
  *(pest: `command` requires a named command inside the parens)*

  `tests/grammar/invalid/syntax/unknown_tagged_literal.edn`:
  ```edn
  (transact [[#ref "550e8400-e29b-41d4-a716-446655440000" :entity/name "test"]])
  ```
  *(pest: only `#uuid` is a valid tagged literal)*

  `tests/grammar/invalid/syntax/string_unterminated.edn`:
  ```edn
  (transact [[:alice :person/name "Alice]])
  ```
  *(the string is never closed — `])` are consumed as string characters)*

  `tests/grammar/invalid/syntax/bind_slot_empty.edn`:
  ```edn
  (query [:find ?name :where [?e :person/name $]])
  ```
  *(`$` not followed by an identifier)*

  `tests/grammar/invalid/syntax/keyword_invalid_chars.edn`:
  ```edn
  (transact [[:alice :person@name "Alice"]])
  ```
  *(`@` is not a valid keyword character — tokenizer emits "Unexpected character: @")*

  `tests/grammar/invalid/syntax/unexpected_bare_char.edn`:
  ```edn
  (transact [[:alice :person/name @bad]])
  ```
  *(`@` outside a string triggers a lex error)*

  `tests/grammar/invalid/syntax/not_join_no_clauses.edn`:
  ```edn
  (query [:find ?name
          :where [?e :person/name ?name]
                 (not-join [?e])])
  ```
  *(`not_join_clause` in grammar requires `where_clause+` — zero clauses is a syntax error)*

- [ ] **Step 2: Run the syntax corpus test — expect all 10 to be rejected by both parsers**

  ```bash
  cargo test grammar_conformance::invalid_syntax_corpus -- --nocapture
  ```

  Expected: all 10 pass (both pest and parser reject each file). Fix any that don't:
  - If pest accepts a file: the grammar is too permissive for this case — tighten it or move the file to `invalid/semantic/`.
  - If the parser accepts a file: the file content is valid — fix the content or move to `valid/`.

- [ ] **Step 3: Commit**

  ```bash
  git add tests/grammar/invalid/syntax/
  git commit -m "test: add invalid/syntax corpus (Task 9)"
  ```

---

## Task 10: `invalid/semantic/` corpus

**Files:**
- Create: `tests/grammar/invalid/semantic/not_safety_unbound_var.edn`
- Create: `tests/grammar/invalid/semantic/not_nested_inside_not.edn`
- Create: `tests/grammar/invalid/semantic/or_inside_not.edn`
- Create: `tests/grammar/invalid/semantic/not_join_unbound_join_var.edn`
- Create: `tests/grammar/invalid/semantic/expr_unbound_var_filter.edn`
- Create: `tests/grammar/invalid/semantic/aggregate_var_unbound.edn`
- Create: `tests/grammar/invalid/semantic/with_without_aggregate.edn`
- Create: `tests/grammar/invalid/semantic/with_var_unbound.edn`
- Create: `tests/grammar/invalid/semantic/window_only_func_without_over.edn`
- Create: `tests/grammar/invalid/semantic/window_incompatible_func_with_over.edn`
- Create: `tests/grammar/invalid/semantic/fact_too_few_elements.edn`
- Create: `tests/grammar/invalid/semantic/retract_wrong_arity.edn`
- Create: `tests/grammar/invalid/semantic/invalid_uuid_format.edn`

- [ ] **Step 1: Write all 13 semantic-error corpus files**

  `tests/grammar/invalid/semantic/not_safety_unbound_var.edn`:
  ```edn
  (query [:find ?name
          :where [?e :person/name ?name]
                 (not [?unbound :person/banned true])])
  ```
  *(`?unbound` is not bound by any outer clause — parser: "not bound by any outer clause")*

  `tests/grammar/invalid/semantic/not_nested_inside_not.edn`:
  ```edn
  (query [:find ?name
          :where [?e :person/name ?name]
                 (not (not [?e :person/banned true]))))
  ```
  *(pest accepts nested `not`; parser: "(not ...) cannot appear inside another (not ...)")*

  `tests/grammar/invalid/semantic/or_inside_not.edn`:
  ```edn
  (query [:find ?name
          :where [?e :person/name ?name]
                 (not (or [?e :person/city "London"]
                          [?e :person/city "Paris"]))))
  ```
  *(pest accepts `or` inside `not`; parser: "(or)/(or-join) cannot appear inside (not)/(not-join)")*

  `tests/grammar/invalid/semantic/not_join_unbound_join_var.edn`:
  ```edn
  (query [:find ?name
          :where [?e :person/name ?name]
                 (not-join [?unbound]
                   [?unbound :person/banned true])])
  ```
  *(`?unbound` is not bound by outer clauses — parser: "join variable ... not bound")*

  `tests/grammar/invalid/semantic/expr_unbound_var_filter.edn`:
  ```edn
  (query [:find ?name
          :where [?e :person/name ?name]
                 [(> ?salary 50000)]])
  ```
  *(`?salary` used in expr before being bound — parser: "not bound by any earlier clause")*

  `tests/grammar/invalid/semantic/aggregate_var_unbound.edn`:
  ```edn
  (query [:find (sum ?salary)
          :where [?e :person/name ?name]])
  ```
  *(`?salary` in aggregate is not bound in `:where` — parser: "Aggregate variable ... not bound")*

  `tests/grammar/invalid/semantic/with_without_aggregate.edn`:
  ```edn
  (query [:find ?name
          :with ?e
          :where [?e :person/name ?name]])
  ```
  *(':with' requires at least one aggregate in :find — parser error)*

  `tests/grammar/invalid/semantic/with_var_unbound.edn`:
  ```edn
  (query [:find (count ?e)
          :with ?unbound
          :where [?e :person/name ?name]])
  ```
  *(`?unbound` in `:with` not bound in `:where` — parser: "':with' variable ... not bound")*

  `tests/grammar/invalid/semantic/window_only_func_without_over.edn`:
  ```edn
  (query [:find (avg ?salary)
          :where [?e :emp/salary ?salary]])
  ```
  *(pest: `aggregate_expr` matches `(avg ?salary)` structurally; parser: "avg is a window function and requires an ':over (...)' clause")*

  `tests/grammar/invalid/semantic/window_incompatible_func_with_over.edn`:
  ```edn
  (query [:find (count-distinct ?e :over (:order-by ?name))
          :where [?e :person/name ?name]])
  ```
  *(pest: `window_expr` matches the structure; parser: "count-distinct is not window-compatible and cannot be used with ':over'")*

  `tests/grammar/invalid/semantic/fact_too_few_elements.edn`:
  ```edn
  (transact [[:alice :person/name]])
  ```
  *(pest: `fact = "[" ~ edn_value+` matches 2 elements; parser: "must have at least 3 elements")*

  `tests/grammar/invalid/semantic/retract_wrong_arity.edn`:
  ```edn
  (retract [[:alice :person/name "Alice" "extra"]])
  ```
  *(pest: `fact = "[" ~ edn_value+` accepts 4 elements; parser: "Optional 4th element must be a map")*

  `tests/grammar/invalid/semantic/invalid_uuid_format.edn`:
  ```edn
  (transact [[#uuid "not-a-valid-uuid" :entity/name "test"]])
  ```
  *(pest: `uuid_lit` accepts any string after `#uuid`; parser validates RFC 4122 format)*

- [ ] **Step 2: Run the semantic corpus test — expect all 13 to be: pest accepts + parser rejects**

  ```bash
  cargo test grammar_conformance::invalid_semantic_corpus -- --nocapture
  ```

  Expected: all 13 pass. Fix any failures:
  - If pest rejects a file: the grammar is too strict — loosen it or move the file to `invalid/syntax/`.
  - If parser accepts a file: the file content is actually valid — fix the content.

- [ ] **Step 3: Run the full conformance suite to confirm all three buckets pass together**

  ```bash
  cargo test grammar_conformance -- --nocapture
  ```

  Expected: all three test functions pass.

- [ ] **Step 4: Commit**

  ```bash
  git add tests/grammar/invalid/semantic/
  git commit -m "test: add invalid/semantic corpus (Task 10)"
  ```

---

## Task 11: EBNF grammar and semantics wiki documentation

**Files:**
- Modify: `.wiki/Datalog-Reference.md` (prepend EBNF grammar + semantics sections; keep existing content below)

- [ ] **Step 1: Prepend the EBNF and semantics sections to `.wiki/Datalog-Reference.md`**

  Read the current file first, then prepend the following content above the existing text. Keep all existing content intact below the new sections.

  New content to prepend:

  ````markdown
  # Datalog Reference

  Minigraf uses a Datalog dialect with EDN (Extensible Data Notation) syntax.
  The machine-checkable version of this grammar is `tests/grammar/grammar.pest`.

  ---

  ## Formal Grammar (EBNF)

  The following EBNF specifies the **structural** syntax accepted by the parser.
  Semantic constraints (safety checks, binding rules, compatibility rules) are documented
  separately in the [Semantic Constraints](#semantic-constraints) section below.

  ```ebnf
  (* ── Top level ────────────────────────────────────────────────────────── *)
  command ::= transact-cmd | retract-cmd | query-cmd | rule-cmd

  (* ── Transact / Retract ───────────────────────────────────────────────── *)
  transact-cmd   ::= "(" "transact" (valid-time-map fact-vector | fact-vector) ")"
  retract-cmd    ::= "(" "retract" fact-vector ")"
  fact-vector    ::= "[" fact* "]"
  fact           ::= "[" edn-value+ "]"
  valid-time-map ::= "{" (":valid-from" string | ":valid-to" string)* "}"

  (* ── Query ────────────────────────────────────────────────────────────── *)
  query-cmd    ::= "(" "query" query-vector ")"
  query-vector ::= "[" query-section+ "]"
  query-section ::=
      find-section | where-section | as-of-section |
      valid-at-section | any-valid-time-section | with-section

  find-section           ::= ":find" find-spec+
  where-section          ::= ":where" where-clause+
  as-of-section          ::= ":as-of" (integer | string | bind-slot)
  valid-at-section       ::= ":valid-at" (string | ":any-valid-time" | bind-slot)
  any-valid-time-section ::= ":any-valid-time"
  with-section           ::= ":with" variable+

  (* ── Find specs ───────────────────────────────────────────────────────── *)
  find-spec      ::= variable | aggregate-expr | window-expr
  aggregate-expr ::= "(" symbol variable ")"
  window-expr    ::= "(" symbol (variable ":over" | ":over") over-clause ")"
  over-clause    ::= "(" over-option* ")"
  over-option    ::= (":partition-by" | ":order-by") variable | ":desc" | ":asc"

  (* ── Where clauses ────────────────────────────────────────────────────── *)
  where-clause ::=
      pattern-clause | expr-clause | not-clause | not-join-clause |
      or-clause | or-join-clause | rule-invocation

  pattern-clause  ::= "[" edn-value edn-value edn-value "]"
  expr-clause     ::= "[" expr variable? "]"
  not-clause      ::= "(" "not" where-clause+ ")"
  not-join-clause ::= "(" "not-join" join-vars where-clause+ ")"
  or-clause       ::= "(" "or" or-branch+ ")"
  or-join-clause  ::= "(" "or-join" join-vars or-branch+ ")"
  join-vars       ::= "[" variable* "]"
  or-branch       ::= and-branch | where-clause
  and-branch      ::= "(" "and" where-clause+ ")"
  rule-invocation ::= "(" symbol edn-value* ")"

  (* ── Expressions ──────────────────────────────────────────────────────── *)
  expr        ::= "(" (unary-op | symbol) expr-arg ")"
                | "(" binary-op expr-arg expr-arg ")"
  unary-op    ::= "string?" | "integer?" | "float?" | "boolean?" | "nil?"
  binary-op   ::= "<" | ">" | "<=" | ">=" | "=" | "!=" | "+" | "-" | "*" | "/"
                | "starts-with?" | "ends-with?" | "contains?" | "matches?"
  expr-arg    ::= expr | boolean | nil | integer | float | string
                | keyword | variable | bind-slot

  (* ── Rule ─────────────────────────────────────────────────────────────── *)
  rule-cmd    ::= "(" "rule" rule-vector ")"
  rule-vector ::= "[" rule-head where-clause* "]"
  rule-head   ::= "(" symbol variable* ")"

  (* ── EDN values ───────────────────────────────────────────────────────── *)
  edn-value ::= uuid | boolean | nil | float | integer | string | bind-slot
              | keyword | symbol | list | vector | map
  list      ::= "(" edn-value* ")"
  vector    ::= "[" edn-value* "]"
  map       ::= "{" (edn-value edn-value)* "}"

  (* ── Primitives ───────────────────────────────────────────────────────── *)
  keyword   ::= ":" (letter | digit | "/" | "-" | "_")+
  symbol    ::= (letter | "_") (letter | digit | "?" | "_" | "-" | "/")*
              | "-" (letter | digit | "?" | "_" | "-" | "/")+
  variable  ::= "?" (letter | digit | "?" | "_" | "-" | "/")*
  boolean   ::= "true" | "false"
  nil       ::= "nil"
  integer   ::= "-"? digit+
  float     ::= "-"? digit+ "." digit*
  string    ::= '"' str-char* '"'
  str-char  ::= "\" ("n" | "t" | "r" | '"' | "\") | any-char-except-quote-backslash
  uuid      ::= "#uuid" string
  bind-slot ::= "$" (letter | digit | "_" | "-")+

  (* letter = [a-zA-Z], digit = [0-9] *)
  (* Whitespace (space, tab, newline, comma) is ignored between tokens. *)
  ```

  ---

  ## Semantic Constraints

  The following constraints are enforced by the parser **above** the structural grammar layer.
  A syntactically valid input may be rejected for violating one of these rules.

  ### Not-safety

  Every variable referenced in a `(not ...)` body must be bound by an outer clause
  appearing **before** the `not` in the same `:where` or rule body:

  ```datalog
  ;; INVALID — ?banned is not bound before the (not ...)
  (query [:find ?name
          :where [?e :person/name ?name]
                 (not [?banned :person/role :admin])])   ; ?banned unbound

  ;; VALID — ?e is bound by the outer pattern
  (query [:find ?name
          :where [?e :person/name ?name]
                 (not [?e :person/banned true])])
  ```

  For `not-join`, every variable listed in the **join-vars vector** must be bound by an outer clause.
  Variables that appear only in the `not-join` body but are **not** in the join-vars are
  existentially quantified and do not need prior binding:

  ```datalog
  ;; VALID — ?e is the join var (bound above); ?dept is existential (body-only)
  (not-join [?e]
    [?e :dept ?dept]
    [?dept :status :bad])
  ```

  ### Nested not

  `(not ...)` cannot appear directly inside another `(not ...)` or `(not-join ...)`.
  `(or ...)` and `(or-join ...)` cannot appear inside `(not ...)` or `(not-join ...)`.

  ### Expression variable binding

  All variables referenced in an expression filter `[(expr)]` must be bound by an **earlier**
  clause in the same `:where` or rule body (forward-pass check):

  ```datalog
  ;; INVALID — ?salary used before it is bound
  (query [:find ?name
          :where [?e :person/name ?name]
                 [(> ?salary 50000)]          ; ?salary not yet bound
                 [?e :emp/salary ?salary]])

  ;; VALID — ?salary bound before the filter
  (query [:find ?name
          :where [?e :person/name ?name]
                 [?e :emp/salary ?salary]
                 [(> ?salary 50000)]])
  ```

  A binding expression `[(expr) ?out]` adds `?out` to the bound set for subsequent clauses.

  ### Aggregate and `:with` binding

  - Every variable appearing in an aggregate `(count ?x)` must be bound in `:where`.
  - Every variable in `:with` must be bound in `:where`.
  - `:with` requires at least one aggregate in `:find`.

  ### Window function compatibility

  | Function | Requires `:over` | Allowed without `:over` |
  |----------|-----------------|------------------------|
  | `avg`, `rank`, `row-number` | Yes | No |
  | `count-distinct`, `sum-distinct` | No | Yes (not window-compatible) |
  | `count`, `sum`, `min`, `max` | Optional | Yes |
  | UDF names | Optional | Yes (runtime-resolved) |

  ### UUID and timestamp validation

  - `#uuid "..."` — the string must be a valid RFC 4122 UUID (e.g. `"550e8400-e29b-41d4-a716-446655440000"`).
  - `:as-of "..."`, `:valid-at "..."`, `:valid-from "..."`, `:valid-to "..."` — the string must be a
    parseable ISO 8601 UTC timestamp (e.g. `"2024-01-15T10:00:00Z"`).
  - `:as-of N` — the integer counter must be non-negative.

  ### Bind slots in attribute position

  `$slot` is not permitted in the **attribute** position of a pattern when the query is
  used with `db.prepare()`. The query optimizer selects an index based on the attribute at
  prepare time and cannot handle a parameterised attribute.

  ---

  ````

- [ ] **Step 2: Run the full test suite to confirm no regressions**

  ```bash
  cargo test -- --nocapture 2>&1 | tail -5
  ```

  Expected: all tests pass (count ≥ 795 + 3 new grammar conformance tests).

- [ ] **Step 3: Commit the wiki change in the wiki repo, then commit the main repo**

  ```bash
  cd .wiki
  git add Datalog-Reference.md
  git commit -m "docs: add EBNF grammar specification and semantics constraints"
  git push
  cd ..
  git add .wiki
  git commit -m "docs: publish EBNF grammar and semantics to Datalog-Reference wiki (Task 11)"
  ```

---

## Self-Review Checklist

Spec section → task coverage:

| Spec deliverable | Task |
|---|---|
| `tests/grammar/grammar.pest` | Tasks 2–8 |
| `valid/` corpus (31 files) | Tasks 2–8 |
| `invalid/syntax/` corpus (10 files) | Task 9 |
| `invalid/semantic/` corpus (13 files) | Task 10 |
| `tests/grammar_conformance.rs` | Task 1 |
| `Cargo.toml` pest dev-dep | Task 1 |
| EBNF grammar document | Task 11 |
| Semantics documentation | Task 11 |

No placeholders. All code blocks are complete. Type names and function names are consistent across tasks (`DatalogGrammar`, `Rule::command`, `pest_accepts`, `parser_accepts`, `load_corpus`, `valid_corpus`, `invalid_syntax_corpus`, `invalid_semantic_corpus`).
