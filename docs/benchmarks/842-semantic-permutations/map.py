#!/usr/bin/env python3
"""Map #842's excluded semantic families to correctness tests (standard library only).

Usage: python3 -I map.py <repo root> <hooks list> <plain list> <out dir> [targets.csv]

<hooks list> and <plain list> are `cargo test [--features test-hooks] -- --list
--format terse` output with stderr included (2>&1), so each binary's
"Running" line precedes its tests.

For every Rust function in tests/*.rs and src/**/*.rs, the function body is
read with strings, raw strings, char literals and comments skipped, so EDN
braces inside query text do not end it. A function's tags are the regex tags
of its own body plus those of every same-file function it calls, transitively
(helpers such as plan_independence_test's `same` carry the forced-plan loops).

Tests are the names cargo lists; each is matched to the one function with
that name in the file its binary or module path names. Writes tests.csv
(one row per listed test) and summary.json (counts per family and file).
Tags are lexical evidence for review, not proof that a test asserts a contract.

With targets.csv (family, cell, file, test), every curated target must be one
listed test defined in that file; targets.json records each target's tags and
whether it exists only in the test-hooks build. A missing target exits 1.
"""
import csv
import json
import os
import re
import sys
from collections import Counter, defaultdict

TAGS = {
    # Numeric aggregate semantics (C20).
    "num_avg": r"\(avg[ )]",
    "num_sum": r"\(sum[ )]",
    "num_sum_distinct": r"\(sum-distinct[ )]",
    "num_extrema": r"\((min|max)[ )]",
    "num_signed_zero_nan": r"-0\.0|f64::NAN|\bNaN\b|\bnan\b|INFINITY",
    # Bind semantics (C34).
    "bind_prepare": r"\.prepare\(|prepare_query\(",
    "bind_slot": r"\$[a-z][a-z0-9_-]*",
    "bind_entity": r"BindValue::Entity",
    "bind_val_ref": r"BindValue::Val\(\s*Value::Ref",
    "bind_set": r"BindValue::Val\(\s*Value::Set|ValueSet",
    "bind_time": r"BindValue::(TxCount|Timestamp|AnyValidTime)|:as-of \$|:valid-at \$",
    "bind_count": r":limit \$|:offset \$",
    # Cross-kind joins and per-fact metadata (C15/C16/C35).
    "join_keyword_ref": r"\(!?= \?[\w-]+ :[\w/-]+\)|\(!?= :[\w/-]+ \?[\w-]+\)|keyword_(or_)?(equals_)?ref|ref_to_its_entity|keyword_and_ref",
    "join_var_attr": r"\[\?[\w-]+ \?[\w-]+ \?[\w-]+\]",
    "join_pseudo": r":db/(tx-count|tx-id|tx-time|valid-at|valid-from|valid-to)",
    "join_not": r"\(not |not-join",
    "join_or": r"\(or |\(or-join",
    "rule_def": r"\(rule \[|\(rule\b",
    "data_committed": r"\.checkpoint\(\)|Minigraf::open|OpenOptions",
    # Forced planner strategies (test-hooks builds).
    "force_subplans": r"__subplans\(|set_subplans\(",
    "force_subplans_hash": r"Subplans::Hash",
    "force_subplans_correlated": r"Subplans::Correlated",
    "force_reorder": r"__reorder\(|set_reorder\(",
    "force_input_order": r"__input_order\(|set_input_order\(",
    "answer_oracle": r"answers::check\(",
}
FAMILIES = {
    "numeric": [t for t in TAGS if t.startswith("num_")],
    "bind": [t for t in TAGS if t.startswith("bind_")],
    "cross_kind": [t for t in TAGS if t.startswith("join_")],
    "forced": [t for t in TAGS if t.startswith("force_")],
}
COMPILED = {k: re.compile(v) for k, v in TAGS.items()}
FN = re.compile(r"\bfn\s+([A-Za-z_][A-Za-z0-9_]*)")
CALL = re.compile(r"\b([A-Za-z_][A-Za-z0-9_]*)\s*(?:::<[^>]*>)?\s*\(")


def strip(src):
    """Code with comments removed and literal bodies blanked; strings kept separately."""
    code, lits, i, n = [], [], 0, len(src)
    while i < n:
        c = src[i]
        if src.startswith("//", i):
            j = src.find("\n", i)
            i = n if j < 0 else j
        elif src.startswith("/*", i):
            depth, i = 1, i + 2
            while i < n and depth:
                if src.startswith("/*", i):
                    depth, i = depth + 1, i + 2
                elif src.startswith("*/", i):
                    depth, i = depth - 1, i + 2
                else:
                    i += 1
        elif c == "r" and re.match(r'r#*"', src[i:i + 300]) and not (i and (src[i - 1].isalnum() or src[i - 1] == "_")):
            m = re.match(r'r(#*)"', src[i:])
            end = '"' + m.group(1)
            j = src.find(end, i + len(m.group(0)))
            j = n if j < 0 else j
            lits.append((len("".join(code)), src[i:j + len(end)]))
            code.append('""')
            i = j + len(end)
        elif c == '"' or (c == "b" and src.startswith('b"', i) and not (i and src[i - 1].isalnum())):
            j = i + (2 if c == "b" else 1)
            while j < n and src[j] != '"':
                j += 2 if src[j] == "\\" else 1
            lits.append((len("".join(code)), src[i:j + 1]))
            code.append('""')
            i = j + 1
        elif c == "'":
            if src.startswith("'\\", i):
                j = src.find("'", i + 2)
                code.append("' '")
                i = j + 1
            elif i + 2 < n and src[i + 2] == "'":
                code.append("' '")
                i += 3
            else:
                code.append(c)
                i += 1
        else:
            code.append(c)
            i += 1
    return "".join(code), lits


def functions(path):
    """{name: [text of each definition with literals restored]} for one file."""
    src = open(path, encoding="utf-8").read()
    code, lits = strip(src)
    # Rebuild a lookup from code offset to literal text.
    lit_at = defaultdict(list)
    for off, text in lits:
        lit_at[off].append(text)
    offsets = sorted(lit_at)
    # In src/, only test code is read: a test module file (tests.rs, *_tests.rs,
    # or under a tests/ directory) whole, otherwise what follows the first
    # #[cfg(test)]. Production functions would otherwise lend their tags to
    # every test that calls them.
    rel = path.replace(os.sep, "/")
    first = 0
    if "/src/" in rel:
        base = os.path.basename(rel)
        if not (base == "tests.rs" or base.endswith("_tests.rs") or "/tests/" in rel.split("/src/", 1)[1]):
            # A file with no test marker holds no listed test, or is a test
            # module declared under cfg(test) by its parent (fault_matrix.rs).
            marker = re.search(r"#\[cfg\((all\()?test\b", code)
            first = marker.start() if marker else 0
    out = defaultdict(list)
    for m in FN.finditer(code):
        if m.start() < first:
            continue
        start = code.find("{", m.end())
        semi = code.find(";", m.end())
        if start < 0 or (0 <= semi < start):
            continue
        depth, j = 0, start
        while j < len(code):
            if code[j] == "{":
                depth += 1
            elif code[j] == "}":
                depth -= 1
                if depth == 0:
                    break
            j += 1
        body = code[m.start():j + 1]
        literal = " ".join(t for o in offsets if m.start() <= o <= j for t in lit_at[o])
        out[m.group(1)].append(body + "\n" + literal)
    return out


def tags_of(text):
    return {k for k, r in COMPILED.items() if r.search(text)}


def file_tags(path):
    fns = functions(path)
    own = {name: set().union(*(tags_of(t) for t in texts)) for name, texts in fns.items()}
    calls = {name: {c for t in texts for c in CALL.findall(t) if c in fns and c != name} for name, texts in fns.items()}
    closed = {}
    for name in fns:
        seen, stack, acc = {name}, [name], set()
        while stack:
            f = stack.pop()
            acc |= own[f]
            for g in calls[f] - seen:
                seen.add(g)
                stack.append(g)
        closed[name] = acc
    return closed


def candidates(root, test):
    """Files that may define a listed test; first is preferred."""
    binary, _, path = test.partition(" ")
    parts = path.split("::")
    if binary.startswith("tests/"):
        common = os.path.join(root, "tests", "common")
        extra = sorted(os.path.join(common, f) for f in os.listdir(common) if f.endswith(".rs")) if os.path.isdir(common) else []
        return [os.path.join(root, binary)] + extra
    mods = parts[:-1]
    out = []
    while mods:
        base = os.path.join(root, "src", *mods)
        out += [base + ".rs", os.path.join(base, "mod.rs")]
        mods = mods[:-1]
    out.append(os.path.join(root, "src", "lib.rs"))
    return [p for p in out if os.path.exists(p)]


def listed(path):
    """(binary, test path) pairs from `cargo test -- --list --format terse` output.

    cargo prints each binary's tests after a 'Running <path>' line on stderr;
    callers pass a file that interleaves both (2>&1)."""
    binary, out = None, []
    for line in open(path, encoding="utf-8"):
        line = line.rstrip("\n")
        m = re.search(r"Running (unittests )?(\S+)", line)
        if m:
            src = m.group(2)
            binary = src if src.startswith("tests/") else "lib" if src.startswith("src/lib") else src
            continue
        if re.search(r"Doc-tests", line):
            binary = "doc"
            continue
        if line.endswith(": test") and binary:
            out.append((binary, line[: -len(": test")]))
    return out


def main(root, list_path, plain_path, out_dir, targets_path=None):
    cache = {}
    rows, unmatched = [], []
    for binary, test in listed(list_path):
        if binary == "doc" or not (binary == "lib" or binary.startswith("tests/")):
            continue
        name = test.split("::")[-1]
        key = f"{binary} {test}" if binary.startswith("tests/") else f"lib {test}"
        files = candidates(root, key if binary.startswith("tests/") else "src " + test)
        hit = None
        for f in files:
            if f not in cache:
                cache[f] = file_tags(f)
            if name in cache[f]:
                hit = f
                break
        if hit is None:
            unmatched.append(key)
            continue
        tags = cache[hit][name]
        fams = [fam for fam, ts in FAMILIES.items() if tags & set(ts)]
        rows.append({
            "binary": binary,
            "test": test,
            "file": os.path.relpath(hit, root),
            "families": " ".join(fams),
            "tags": " ".join(sorted(tags)),
        })
    os.makedirs(out_dir, exist_ok=True)
    with open(os.path.join(out_dir, "tests.csv"), "w", newline="", encoding="utf-8") as fh:
        w = csv.DictWriter(fh, fieldnames=["binary", "test", "file", "families", "tags"], lineterminator="\n")
        w.writeheader()
        w.writerows(sorted(rows, key=lambda r: (r["file"], r["test"])))
    summary = {"listed_rust_tests": len(rows) + len(unmatched), "matched": len(rows),
               "unmatched": sorted(unmatched), "families": {}, "tags": {}}
    for fam in FAMILIES:
        per_file = Counter(r["file"] for r in rows if fam in r["families"].split())
        summary["families"][fam] = {"tests": sum(per_file.values()), "files": dict(sorted(per_file.items()))}
    for t in TAGS:
        summary["tags"][t] = sum(1 for r in rows if t in r["tags"].split())
    with open(os.path.join(out_dir, "summary.json"), "w", encoding="utf-8") as fh:
        json.dump(summary, fh, indent=2, sort_keys=True)
        fh.write("\n")
    print(json.dumps({k: summary[k] for k in ("listed_rust_tests", "matched")}),
          {f: v["tests"] for f, v in summary["families"].items()}, f"unmatched={len(unmatched)}")
    if targets_path:
        plain = {(b, t) for b, t in listed(plain_path)}
        index = defaultdict(list)
        for r in rows:
            index[(r["file"], r["test"].split("::")[-1])].append(r)
        checked, missing = [], []
        for t in csv.DictReader(open(targets_path, encoding="utf-8")):
            hits = index.get((t["file"], t["test"]), [])
            if len(hits) != 1:
                missing.append(f'{t["file"]} {t["test"]} ({len(hits)} matches)')
                continue
            r = hits[0]
            checked.append({**t, "listed_as": f'{r["binary"]} {r["test"]}',
                            "tags": r["tags"],
                            "needs_test_hooks": (r["binary"], r["test"]) not in plain})
        cells = defaultdict(lambda: {"tests": 0, "needs_test_hooks": 0})
        for c in checked:
            cells[f'{c["family"]}: {c["cell"]}']["tests"] += 1
            cells[f'{c["family"]}: {c["cell"]}']["needs_test_hooks"] += int(c["needs_test_hooks"])
        with open(os.path.join(out_dir, "targets.json"), "w", encoding="utf-8") as fh:
            json.dump({"targets": len(checked), "missing": missing, "cells": dict(sorted(cells.items())),
                       "checked": checked}, fh, indent=2)
            fh.write("\n")
        print(f"targets={len(checked)} missing={len(missing)}", *missing, sep="\n  ")
        if missing:
            sys.exit(1)


if __name__ == "__main__":
    main(*sys.argv[1:6])
