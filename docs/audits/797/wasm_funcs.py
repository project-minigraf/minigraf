#!/usr/bin/env python3
"""Per-function code sizes of an unstripped Wasm module, by name (#797).

Usage: wasm_funcs.py BASE.wasm CANDIDATE.wasm
Prints the functions whose body size differs, grouped by name with the Rust
symbol hash removed. Attribution only: the stripped artifact is the measure.
"""
import re
import sys
from collections import Counter
from pathlib import Path


def leb(data, pos):
    value, shift = 0, 0
    while True:
        byte = data[pos]
        pos += 1
        value |= (byte & 127) << shift
        if byte < 128:
            return value, pos
        shift += 7


def functions(path):
    data = Path(path).read_bytes()
    pos, imported, bodies, names = 8, 0, [], {}
    while pos < len(data):
        kind = data[pos]
        size, pos = leb(data, pos + 1)
        end = pos + size
        if kind == 2:
            count, p = leb(data, pos)
            for _ in range(count):
                for _ in range(2):
                    n, p = leb(data, p)
                    p += n
                desc = data[p]
                p += 1
                if desc == 0:
                    _, p = leb(data, p)
                    imported += 1
                elif desc == 1:
                    p += 1
                    flags, p = leb(data, p)
                    _, p = leb(data, p)
                    if flags & 1:
                        _, p = leb(data, p)
                elif desc == 2:
                    flags, p = leb(data, p)
                    _, p = leb(data, p)
                    if flags & 1:
                        _, p = leb(data, p)
                elif desc == 3:
                    p += 2
                else:
                    p += 2
        elif kind == 10:
            count, p = leb(data, pos)
            for _ in range(count):
                n, p = leb(data, p)
                bodies.append(n)
                p += n
        elif kind == 0:
            n, p = leb(data, pos)
            if data[p:p + n] == b"name":
                p += n
                while p < end:
                    sub = data[p]
                    sz, p = leb(data, p + 1)
                    if sub == 1:
                        q = p
                        count, q = leb(data, q)
                        for _ in range(count):
                            idx, q = leb(data, q)
                            ln, q = leb(data, q)
                            names[idx] = data[q:q + ln].decode("utf-8", "replace")
                            q += ln
                    p += sz
        pos = end
    out = Counter()
    for i, n in enumerate(bodies):
        name = names.get(imported + i, f"<{imported + i}>")
        out[re.sub(r"17h[0-9a-f]{16}E", "", name)] += n
    return out


base, cand = functions(sys.argv[1]), functions(sys.argv[2])
rows = sorted(set(base) | set(cand), key=lambda k: cand[k] - base[k])
total = 0
for k in rows:
    d = cand[k] - base[k]
    if d:
        total += d
        print(f"{d:+7d} {base[k]:7d} {cand[k]:7d} {k}")
print(f"{total:+7d} total code bytes")
