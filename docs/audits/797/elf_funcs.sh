#!/usr/bin/env bash
# Per-symbol size differences of two unstripped ELF builds (#797), with Rust
# symbol hashes removed. Attribution only: the stripped artifact is the measure.
# Usage: elf_funcs.sh BASE CANDIDATE
set -euo pipefail
sizes() {
    nm -S --size-sort -C "$1" | awk '{size=strtonum("0x"$2); $1=$2=$3=""; sub(/^ +/, ""); gsub(/::h[0-9a-f]{16}$/, ""); s[$0]+=size} END {for (k in s) print s[k] "\t" k}' | sort -t$'\t' -k2
}
join -t$'\t' -1 2 -2 2 -a 1 -a 2 -e 0 -o 0,1.1,2.1 <(sizes "$1") <(sizes "$2") \
    | awk -F'\t' '$2 != $3 {d=$3-$2; t+=d; printf "%+7d %7d %7d %s\n", d, $2, $3, $1} END {printf "%+7d total symbol bytes\n", t}' \
    | sort -n
