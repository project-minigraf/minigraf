"""Manual serial runner. Launch on the idle benchmark host, never in nightly CI."""
import argparse
from collections import Counter
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

from cases import matrix
from evidence import summarize


def cpu():
    values = list(map(int, Path("/proc/stat").read_text().splitlines()[0].split()[1:9]))
    return sum(values), values[3], values[4]


def idle_samples(count=10):
    def swaps():
        fields = dict(line.split() for line in Path("/proc/vmstat").read_text().splitlines())
        return int(fields["pswpin"]) + int(fields["pswpout"])
    before = cpu()
    before_swap = swaps()
    out = []
    for _ in range(count):
        time.sleep(1)
        after = cpu()
        total = after[0] - before[0]
        current_swap = swaps()
        out.append({"idle_percent": 100*(after[1]-before[1])/total,
                    "iowait_percent": 100*(after[2]-before[2])/total,
                    "swapped_pages": current_swap-before_swap})
        before = after
        before_swap = current_swap
    return out


def memory_available():
    return int(next(l.split()[1] for l in Path("/proc/meminfo").read_text().splitlines()
                    if l.startswith("MemAvailable:"))) * 1024


def memory_caps(nodes):
    """Conservative initial ceilings shared across concurrent children."""
    if not nodes:
        raise ValueError("at least one node required")
    available = memory_available()
    counts = Counter(nodes)
    free = {}
    for node in counts:
        info = Path(f"/sys/devices/system/node/node{node}/meminfo").read_text()
        free[node] = int(next(l.split()[-2] for l in info.splitlines() if "MemFree:" in l))*1024
    caps = [min(available*4//5//len(nodes), free[n]*4//5//counts[n]) for n in nodes]
    return {"host_available_bytes": available, "node_free_bytes": free,
            "nodes": nodes, "child_caps_bytes": caps}


def select_cases(cases, requested):
    if requested is None:
        return cases
    if (not isinstance(requested, list) or not requested
            or not all(isinstance(i, str) for i in requested)
            or len(set(requested)) != len(requested)):
        raise ValueError("case selection must be a nonempty list of unique IDs")
    known = {c["id"] for c in cases}
    if set(requested)-known:
        raise ValueError("unknown or family-excluded case ID")
    return [c for c in cases if c["id"] in set(requested)]


def rss(pid):
    try:
        text = Path(f"/proc/{pid}/status").read_text()
        return int(next(l.split()[1] for l in text.splitlines() if l.startswith("VmRSS:")))*1024
    except (FileNotFoundError, StopIteration):
        return 0


def invoke(command, stem, timeout, memory_cap, disk_floor, scratch):
    start = time.monotonic_ns()
    observed = []
    disposition = None
    last_cpu = cpu()
    busy_intervals = 0
    with stem.with_suffix(".out").open("w") as out, stem.with_suffix(".err").open("w") as err:
        proc = subprocess.Popen(command, stdout=out, stderr=err)
        while proc.poll() is None:
            current = rss(proc.pid)
            current_cpu = cpu()
            cpu_total = current_cpu[0]-last_cpu[0]
            idle = 100*(current_cpu[1]-last_cpu[1])/cpu_total if cpu_total else 100
            last_cpu = current_cpu
            observed.append([time.monotonic_ns()-start, current, idle])
            busy_intervals = busy_intervals+1 if idle<95 else 0
            if (time.monotonic_ns()-start)/1e9 > timeout:
                disposition = "timeout"
            elif current > memory_cap or shutil.disk_usage(scratch).free < disk_floor:
                disposition = "resource_limit"
            elif busy_intervals >= 10:
                disposition = "interference"
            if disposition:
                proc.kill()
                break
            time.sleep(0.1)
        status = proc.wait()
    elapsed = time.monotonic_ns()-start
    stem.with_suffix(".resources.json").write_text(json.dumps(observed))
    if disposition or status:
        return {"disposition": disposition or "error", "exit_status": status,
                "elapsed_lower_bound_ns": elapsed,
                "reason": disposition or stem.with_suffix(".err").read_text()[-3000:]}
    return {"disposition": "measured", "exit_status": 0}


def main():
    p = argparse.ArgumentParser()
    p.add_argument("--binary", type=Path, required=True)
    p.add_argument("--out", type=Path, required=True)
    p.add_argument("--fixtures", type=Path, required=True)
    p.add_argument("--smoke", action="store_true")
    p.add_argument("--prepare-only", action="store_true")
    p.add_argument("--families", default="scalar,compound,window,set,uuid")
    p.add_argument("--sessions", type=int, default=3)
    p.add_argument("--samples", type=int, default=5)
    p.add_argument("--node", default="1")
    p.add_argument("--timeout", type=int, default=600)
    p.add_argument("--core-revision", required=True)
    p.add_argument("--case-ids", type=Path, help="JSON list of explicit case IDs for a separate attempt")
    a = p.parse_args()
    a.binary = a.binary.resolve()
    a.out.mkdir(parents=True, exist_ok=False)
    a.fixtures.mkdir(parents=True, exist_ok=True)
    raw = a.out / "raw"
    raw.mkdir()
    cases = [c for c in matrix(a.smoke) if c["family"] in a.families.split(",")]
    selected = json.loads(a.case_ids.read_text()) if a.case_ids else None
    cases = select_cases(cases, selected)
    if not cases:
        raise ValueError("empty case selection")
    placement = ["numactl", f"--cpunodebind={a.node}", f"--membind={a.node}"]
    bounds = memory_caps([a.node])
    memcap = bounds["child_caps_bytes"][0]
    disk_floor = int(shutil.disk_usage(a.fixtures).free * 0.2)
    preflight = idle_samples()
    (a.out/"run.json").write_text(json.dumps({
        "source_sha": a.core_revision,
        "binary_sha256": hashlib.sha256(a.binary.read_bytes()).hexdigest(),
        "preflight": preflight, "placement": placement, "sessions": a.sessions,
        "samples": a.samples, "timeout_seconds": a.timeout,
        "memory_cap_bytes": memcap, "disk_free_floor_bytes": disk_floor,
        "memory_bounds": bounds, "selected_case_ids": selected,
        "cache_state": "warm OS cache; fresh process/handle and one verified warmup per case/session; samples share handle",
        "smoke": a.smoke, "started_utc": time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),
    }, indent=2))
    if any(r["idle_percent"] < 95 or r["swapped_pages"] for r in preflight):
        raise RuntimeError("host is busy; retained preflight, no measurements started")
    paths = []
    failed = {}
    for i, c in enumerate(cases):
        path = a.out/f"case-{i:03}.json"
        path.write_text(json.dumps(c, indent=2))
        c["query"] = subprocess.check_output([a.binary,"query",path,a.fixtures], text=True).strip()
        path.write_text(json.dumps(c, indent=2))
        paths.append(path)
        command = placement+[str(a.binary),"build",str(path),str(a.fixtures)]
        result = invoke(command, raw/f"build-{i:03}",3600,memcap,disk_floor,a.fixtures)
        if result["disposition"] != "measured":
            failed[c["id"]] = result
        print(f"build {i+1}/{len(cases)} {c['id']} {result['disposition']}", flush=True)
    (a.out/"cases.json").write_text(json.dumps(cases, indent=2))
    if a.prepare_only:
        (a.out/"build-failures.json").write_text(json.dumps(failed, indent=2))
        return
    rows = []
    with (a.out/"samples.jsonl").open("w") as output:
        for session in range(a.sessions):
            idle = idle_samples()
            (a.out/f"session-{session}-idle.json").write_text(json.dumps(idle))
            if any(r["idle_percent"] < 95 or r["swapped_pages"] for r in idle):
                raise RuntimeError("interference before session; retain partial run")
            # Reverse case order every session to expose ordering/cache effects.
            indices = list(range(len(cases)))
            if session % 2:
                indices.reverse()
            for i in indices:
                c = cases[i]
                if c["id"] in failed:
                    batch_result = dict(disposition="deferred", exit_status=None,
                                        reason=f"earlier failed attempt: {failed[c['id']]}")
                    values = None
                else:
                    stem = raw/f"s{session}-c{i:03}"
                    command = placement+[str(a.binary),"batch",str(paths[i]),str(a.fixtures),str(a.samples)]
                    batch_result = invoke(command,stem,a.timeout,memcap,disk_floor,a.fixtures)
                    values = None
                    if batch_result["disposition"] == "measured":
                        values = json.loads(stem.with_suffix(".out").read_text())
                        if not isinstance(values,list) or len(values)!=a.samples:
                            raise ValueError("wrong batch sample count")
                    else:
                        failed[c["id"]] = batch_result.copy()
                for sample in range(a.samples):
                    key = dict(case=c["id"], session=session, sample=sample)
                    result = batch_result.copy()
                    if values is not None:
                        result.update(values[sample])
                        if result["sample_in_process"]!=sample or result["process_samples"]!=a.samples:
                            raise ValueError("wrong in-process sample identity")
                        wal = a.fixtures / (result["fixture"]+".graph.wal")
                        result["wal_bytes"] = wal.stat().st_size if wal.exists() else 0
                        result["total_storage_bytes"] = result["file_bytes"] + result["wal_bytes"]
                    result.update(key)
                    rows.append(result)
                    output.write(json.dumps(result)+"\n")
                    output.flush()
                print(f"session {session+1} case {i+1}/{len(cases)} {c['id']} {result['disposition']}", flush=True)
    (a.out/"summary.json").write_text(json.dumps(summarize(cases,rows,a.sessions,a.samples),indent=2))


if __name__ == "__main__":
    main()
