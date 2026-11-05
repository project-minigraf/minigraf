"""Build matched diagnostic variants, restoring source after EACH build."""
import difflib
import hashlib
import json
from pathlib import Path
import shutil
import subprocess
import sys


VARIANTS = {
    "no-cache": ("src/query/engine/join.rs", [
        ("if ctx.derived.is_some() {", "if false {"),
        ("if ctx.derived.is_none() {", "if true {"),
    ]),
    "no-ancestor-alternative": ("src/query/engine/recursive.rs", [
        ("let Some(alternative) = run.entity_first.as_deref() else {",
         "let Some(alternative) = None::<&RuleRun> else {"),
    ]),
    "attribute-scan": ("src/query/engine/scan.rs", [
        ('''        if let (Some(a), Some(keys)) = (a, ctx.keys) {
            for v in &candidates {
                if let Some(kv) = key_value(keys, v)? {
                    ranges.push((Index::Avet, keys::attribute_value_prefix(a, &kv)));
                }
            }
        }''', '''        if let (Some(a), Some(_)) = (a, ctx.keys) {
            // Audit control: the existing join still checks every join key.
            ranges.push((Index::Aevt, keys::attribute_prefix(a)));
        }'''),
    ]),
    "static-connected-order": ("src/query/datalog/optimizer.rs", [
        ("let score = selectivity_score(p, &bound);", "let score = 0; // Audit: keep initial priority among connected patterns."),
    ]),
}


def build(root):
    root = Path(root).resolve()
    out = root / "evidence" / "variants"
    out.mkdir(exist_ok=False)
    binary = root / "scripts/shape-sweep/native/target/release/shape-sweep"
    manifest = root / "scripts/shape-sweep/native/Cargo.toml"
    provenance = {}
    for name in ["baseline", *VARIANTS]:
        source = None
        original = None
        try:
            if name in VARIANTS:
                relative, replacements = VARIANTS[name]
                source = root / relative
                original = source.read_text()
                changed = original
                for before, after in replacements:
                    if changed.count(before) != 1:
                        raise ValueError(f"ambiguous/missing substitution: {name}")
                    changed = changed.replace(before, after)
                source.write_text(changed)
                patch = "".join(difflib.unified_diff(original.splitlines(True), changed.splitlines(True),
                                                   fromfile="a/"+relative,tofile="b/"+relative))
                (out/f"{name}.patch").write_text(patch)
            with (out/f"{name}-test.log").open("w") as log:
                subprocess.run(["cargo","test","--offline","--locked","--manifest-path",str(manifest),
                                "recursive_ancestors"],cwd=root,stdout=log,stderr=log,check=True)
            with (out/f"{name}-build.log").open("w") as log:
                subprocess.run(["cargo","build","--offline","--locked","--release","--manifest-path",str(manifest)],
                               cwd=root,stdout=log,stderr=log,check=True)
            shutil.copy2(binary,out/name)
            provenance[name] = {"sha256":hashlib.sha256(binary.read_bytes()).hexdigest(),
                                "correctness":"independent chain/tree walks; pending and committed"}
            (out/"builds.json").write_text(json.dumps(provenance,indent=2))
            print(f"built {name}",flush=True)
        finally:
            if source is not None and original is not None:
                source.write_text(original)
    # The normal runner must use the baseline, never the last modified binary.
    shutil.copy2(out/"baseline",binary)


if __name__ == "__main__":
    build(sys.argv[1])
