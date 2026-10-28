#!/usr/bin/env bash
set -euo pipefail
bench_run=${1:?usage: 532-profile.sh RUN_DIR}
export bench_run
test -s "$bench_run/fixtures/fixture-v1-1m.json"
test ! -s "$bench_run/fixtures/fixture-v1-1m.graph.wal"
{
  date -u
  uptime
  ps -eo pid,pcpu,comm --sort=-pcpu | head -15
} > "$bench_run/load-before-profiles.txt"
for spec in attribute_scan:2:1 join_3_selective:4:200 join_3_scan:5:1 recursive_ancestors:10:40; do
  name=${spec%%:*}
  rest=${spec#*:}
  index=${rest%%:*}
  repeats=${rest#*:}
  frequency=499
  # The original 499 Hz recursive capture lost samples; 249 Hz was retried.
  if [ "$name" = recursive_ancestors ]; then frequency=249; fi
  perf record -e cycles:u -F "$frequency" --call-graph dwarf,16384 \
    -o "$bench_run/$name.data" -- \
    numactl --cpunodebind=1 --membind=1 sh -c '
      index=$1
      repeats=$2
      count=0
      while [ "$count" -lt "$repeats" ]; do
        "$bench_run/target/envelope/examples/perf_envelope" __query \
          "$bench_run/fixtures/fixture-v1-1m.graph" "$index" 15 60
        count=$((count + 1))
      done
    ' profile-children "$index" "$repeats" \
    > "$bench_run/$name-children.jsonl" 2> "$bench_run/$name-record.log"
  perf report -i "$bench_run/$name.data" --stdio --header --children --inline \
    2> "$bench_run/$name-report-errors.txt" | c++filt -s rust > "$bench_run/$name-report.txt"
  perf script -i "$bench_run/$name.data" --inline -F comm,pid,tid,time,event,period,ip,sym,dso \
    2> "$bench_run/$name-script-errors.txt" | c++filt -s rust > "$bench_run/$name-stacks.txt"
done
{
  date -u
  uptime
  ps -eo pid,pcpu,comm --sort=-pcpu | head -15
} > "$bench_run/load-after-profiles.txt"
