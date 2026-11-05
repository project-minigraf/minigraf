"""Install audit-only probes in a DISPOSABLE checkout; never edit shipping APIs."""
from pathlib import Path
import difflib


def install(root):
    root = Path(root).resolve()
    edits = {
        "src/db.rs": '\n#[cfg(all(test, not(target_arch = "wasm32")))]\n#[path = "../scripts/shape-sweep/probe.rs"]\nmod shape_sweep;\n',
        "src/graph/storage.rs": '''
#[cfg(test)]
impl FactStorage {
    pub(crate) fn sweep_pin_stats(&self) -> (usize, Option<u64>, usize) {
        self.pins.sweep_stats()
    }
}
''',
        "src/storage/pins.rs": '''
#[cfg(test)]
impl Pins {
    pub(crate) fn sweep_stats(&self) -> (usize, Option<u64>, usize) {
        let s = self.lock();
        (s.pinned.values().sum(), s.pinned.keys().next().copied(),
         s.quarantine.values().map(|v| v.capacity() * std::mem::size_of::<u64>()).sum())
    }
}
''',
    }
    patch = []
    for name, addition in edits.items():
        path = root / name
        text = path.read_text()
        if addition in text:
            raise ValueError(f"already instrumented: {name}")
        path.write_text(text + addition)
        patch.extend(difflib.unified_diff(text.splitlines(True), (text + addition).splitlines(True),
                                         fromfile="a/"+name, tofile="b/"+name))
    (root / "evidence" / "instrumentation.patch").write_text("".join(patch))


if __name__ == "__main__":
    import sys
    install(sys.argv[1])
