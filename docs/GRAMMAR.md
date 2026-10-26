# Datalog Grammar

<!-- Generated from tests/grammar/grammar.pest by tests/grammar_doc.rs. Do not edit: change the grammar, then run UPDATE_GRAMMAR_DOC=1 cargo test --test grammar_doc -->

This EBNF is the structural syntax of Minigraf's Datalog commands. It is generated from `tests/grammar/grammar.pest`, the grammar the parser is tested against: for every input up to a size bound, a test checks that the parser rejects what the grammar rejects, and accepts what it accepts unless a semantic rule rejects it. To change this page, edit the grammar and run `UPDATE_GRAMMAR_DOC=1 cargo test --test grammar_doc`.

Between tokens the parser skips spaces, tabs, newlines, carriage returns and commas. It also skips comments: `;` and the rest of its line. Inside a rule marked `(* token *)` nothing is skipped, and the rules it uses are read the same way.

EBNF has no lookahead, so a pest lookahead appears as a comment: `(* not followed by x *)` means the next characters must not match `x`, and nothing is read. Literals have no escapes: `"\"` is one backslash and `'"'` one double quote.

```ebnf
(* ── WORD-BOUNDARY HELPER ── *)
word-cont            ::= letter | numeral | "?" | "_" | "-" | "/"
kw-find              ::= ":find" (* not followed by word-cont *)  (* token *)
kw-where             ::= ":where" (* not followed by word-cont *)  (* token *)
kw-with              ::= ":with" (* not followed by word-cont *)  (* token *)
kw-as-of             ::= ":as-of" (* not followed by word-cont *)  (* token *)
kw-valid-at          ::= ":valid-at" (* not followed by word-cont *)  (* token *)
kw-any-valid-time    ::= ":any-valid-time" (* not followed by word-cont *)  (* token *)
kw-max-derived-facts ::= ":max-derived-facts" (* not followed by word-cont *)  (* token *)
kw-max-results       ::= ":max-results" (* not followed by word-cont *)  (* token *)
kw-limit             ::= ":limit" (* not followed by word-cont *)  (* token *)
kw-offset            ::= ":offset" (* not followed by word-cont *)  (* token *)
kw-distinct          ::= ":distinct" (* not followed by word-cont *)  (* token *)
kw-over              ::= ":over" (* not followed by word-cont *)  (* token *)
kw-partition-by      ::= ":partition-by" (* not followed by word-cont *)  (* token *)
kw-order-by          ::= ":order-by" (* not followed by word-cont *)  (* token *)
kw-desc              ::= ":desc" (* not followed by word-cont *)  (* token *)
kw-asc               ::= ":asc" (* not followed by word-cont *)  (* token *)
kw-valid-from        ::= ":valid-from" (* not followed by word-cont *)  (* token *)
kw-valid-to          ::= ":valid-to" (* not followed by word-cont *)  (* token *)
reserved-head        ::= ("not-join" | "not" | "or-join" | "or") (* not followed by word-cont *)  (* token *)
and-head             ::= "and" (* not followed by word-cont *)  (* token *)

(* ── EDN PRIMITIVES ── *)
boolean-lit ::= ("true" | "false") (* not followed by word-cont *)  (* token *)
nil-lit     ::= "nil" (* not followed by word-cont *)  (* token *)
integer-lit ::= "-"? digit+ (* not followed by "." digit *)  (* token *)
float-lit   ::= "-"? digit+ "." digit*  (* token *)
str-char    ::= "\" ("n" | "t" | "r" | '"' | "\" | any-char)
              | (* not followed by '"' | "\" *) any-char
string-lit  ::= '"' str-char* '"'  (* token *)
uuid-lit    ::= "#uuid" string-lit
set-elem    ::= uuid-lit | boolean-lit | nil-lit | float-lit | integer-lit | string-lit
              | keyword
set-lit     ::= "#{" set-elem* "}"
bind-slot   ::= "$" (letter | numeral | "_" | "-")+  (* token *)
keyword     ::= ":" word-cont+  (* token *)
variable    ::= "?" word-cont*  (* token *)
op-sym      ::= "<=" | ">=" | "!=" | "<" | ">" | "=" | "+" | "*" | "/"
              | "-" (* not followed by word-cont *)  (* token *)
plain-sym   ::= (* not followed by boolean-lit *) (* not followed by nil-lit *) (letter | "_") word-cont*
              | "-" (* not followed by numeral *) word-cont+  (* token *)

(* ── EDN VALUE (any EDN atom or container) ── *)
edn-value  ::= set-lit | uuid-lit | boolean-lit | nil-lit | float-lit | integer-lit
             | string-lit | bind-slot | keyword | op-sym | variable | plain-sym
             | edn-list | edn-vector | edn-map
edn-list   ::= "(" edn-value* ")"
edn-vector ::= "[" edn-value* "]"
edn-map    ::= "{" (edn-value edn-value)* "}"

(* ── TOP-LEVEL COMMAND ── *)
command ::= transact-cmd | retract-cmd | query-cmd | rule-cmd

(* ── TRANSACT ── *)
transact-cmd ::= "(" "transact" (valid-time-map fact-vector | fact-vector) ")"

(* ── RETRACT ── *)
retract-cmd     ::= "(" "retract" "[" retract-fact* "]" ")"
fact-vector     ::= "[" fact* "]"
fact            ::= "[" edn-value edn-value edn-value valid-time-map? "]"
retract-fact    ::= "[" edn-value edn-value edn-value "]"
valid-time-map  ::= "{" valid-time-pair* "}"
valid-time-pair ::= (kw-valid-from | kw-valid-to) string-lit

(* ── QUERY ── *)
query-cmd     ::= "(" "query" query-vector ")"
query-vector  ::= "[" query-section+ "]"
query-section ::= find-section | where-section | as-of-section | valid-at-section
                | any-valid-time-section | with-section | max-derived-facts-section
                | max-results-section | limit-section | offset-section
                | order-by-section | distinct-section

(* ── FIND SECTION ── *)
find-section   ::= kw-find find-spec+
find-spec      ::= window-expr | aggregate-expr | variable
aggregate-expr ::= "(" (op-sym | plain-sym) variable ")"
window-expr    ::= "(" (op-sym | plain-sym) (variable kw-over | kw-over) over-clause ")"
over-clause    ::= "(" over-option* ")"
over-option    ::= kw-partition-by variable | kw-order-by variable | kw-desc | kw-asc

(* ── WHERE SECTION ── *)
where-section   ::= kw-where where-clause+
where-clause    ::= expr-clause | not-clause | not-join-clause | or-clause
                  | or-join-clause | pattern-clause | rule-invocation
pattern-clause  ::= "[" edn-value edn-value edn-value "]"
expr-clause     ::= "[" (collect-set variable | expr variable?) "]"
not-clause      ::= "(" "not" where-clause+ ")"
not-join-clause ::= "(" "not-join" join-vars where-clause+ ")"
or-clause       ::= "(" "or" or-branch+ ")"
or-join-clause  ::= "(" "or-join" join-vars or-branch+ ")"
join-vars       ::= "[" variable* "]"
or-branch       ::= and-branch | (* not followed by "(" and-head *) where-clause
and-branch      ::= "(" "and" where-clause+ ")"
rule-invocation ::= "(" (* not followed by reserved-head *) plain-sym edn-value* ")"

(* ── EXPRESSIONS ── *)
expr             ::= "(" expr-body ")"
expr-body        ::= binary-form | unary-form
unary-form       ::= (* not followed by collect-set-head *) (unary-op | op-sym | plain-sym) expr-arg
binary-form      ::= binary-op expr-arg expr-arg
collect-set      ::= "(" collect-set-head variable keyword ")"
collect-set-head ::= "collect-set" (* not followed by word-cont *)  (* token *)
unary-op         ::= "string?" | "integer?" | "float?" | "boolean?" | "nil?"  (* token *)
binary-op        ::= "<=" | ">=" | "!=" | "starts-with?" | "ends-with?" | "contains?"
                   | "matches?" | "not-in?" | "any-in?" | "all-in?" | "none-in?" | "in?"
                   | "<" | ">" | "=" | "+" | "-" | "*" | "/"  (* token *)
expr-arg         ::= expr | set-lit | boolean-lit | nil-lit | float-lit | integer-lit
                   | string-lit | bind-slot | keyword | variable

(* ── TEMPORAL / WITH SECTIONS ── *)
as-of-section             ::= kw-as-of (integer-lit | string-lit | bind-slot)
valid-at-section          ::= kw-valid-at (string-lit | kw-any-valid-time | bind-slot)
any-valid-time-section    ::= kw-any-valid-time
with-section              ::= kw-with variable+
max-derived-facts-section ::= kw-max-derived-facts integer-lit
max-results-section       ::= kw-max-results integer-lit
limit-section             ::= kw-limit (integer-lit | bind-slot)
offset-section            ::= kw-offset (integer-lit | bind-slot)
distinct-section          ::= kw-distinct
order-by-section          ::= kw-order-by "[" order-entry+ "]"
order-entry               ::= find-spec (kw-asc | kw-desc)?

(* ── RULE ── *)
rule-cmd    ::= "(" "rule" rule-vector ")"
rule-vector ::= "[" rule-head where-clause* "]"
rule-head   ::= "(" plain-sym variable* ")"

(* letter = a character Unicode calls Alphabetic; numeral = a character in Unicode's number categories (Nd, Nl, No), digits included; digit = "0".."9"; any-char = any one character *)
```

Input that matches this grammar can still be rejected: the parser also enforces the rules in the Semantic Constraints section of the Datalog reference (variable safety, binding, aggregate and window compatibility, UUID and timestamp formats).
