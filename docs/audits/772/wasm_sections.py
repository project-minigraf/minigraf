#!/usr/bin/env python3
"""Print raw Wasm section payload sizes (not a linked-package size estimate)."""
import json
from pathlib import Path
import sys

names = {0: "custom", 1: "type", 2: "import", 3: "function", 4: "table",
         5: "memory", 6: "global", 7: "export", 8: "start", 9: "element",
         10: "code", 11: "data", 12: "data-count", 13: "tag"}
for arg in sys.argv[1:]:
    data = Path(arg).read_bytes()
    assert data[:8] == b"\0asm\1\0\0\0", arg
    pos = 8
    sections = []
    while pos < len(data):
        kind = data[pos]
        pos += 1
        size, shift = 0, 0
        while True:
            byte = data[pos]
            pos += 1
            size |= (byte & 127) << shift
            if byte < 128:
                break
            shift += 7
            assert shift < 35
        assert pos + size <= len(data)
        sections.append({"section": names[kind], "bytes": size})
        pos += size
    print(json.dumps({"artifact": Path(arg).name, "bytes": len(data), "sections": sections}))
