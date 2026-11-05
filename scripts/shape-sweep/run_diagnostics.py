"""Run sustained churn and untimed query observations after public fixtures exist."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess

from run import idle_samples, invoke, memory_caps
import shutil


def checked_lines(path):
    return [json.loads(s.split("SHAPE ",1)[1]) for s in path.read_text().splitlines() if "SHAPE " in s]


def main():
    p=argparse.ArgumentParser()
    p.add_argument("--binary",type=Path,required=True)
    p.add_argument("--out",type=Path,required=True)
    p.add_argument("--fixtures",type=Path,required=True)
    p.add_argument("--public-results",type=Path)
    p.add_argument("--churn",action="store_true")
    p.add_argument("--sessions",type=int,default=3)
    a=p.parse_args()
    a.out.mkdir(parents=True,exist_ok=False)
    a.fixtures.mkdir(parents=True,exist_ok=True)
    prefix=["numactl","--cpunodebind=1","--membind=1",str(a.binary.resolve())]
    bounds=memory_caps(["1"])
    memory_cap=min(bounds["child_caps_bytes"][0],48*1024**3)
    (a.out/"run.json").write_text(json.dumps({"memory_bounds":bounds,"memory_cap_bytes":memory_cap,
        "binary_sha256":hashlib.sha256(a.binary.read_bytes()).hexdigest(),"sessions":a.sessions,
        "churn":a.churn,"placement":prefix[:3]},indent=2))
    disk_floor=int(shutil.disk_usage(a.fixtures).free*0.2)
    records=[]
    preflight=idle_samples()
    (a.out/"preflight.json").write_text(json.dumps(preflight))
    if any(s["idle_percent"]<95 or s["swapped_pages"] for s in preflight):
        raise RuntimeError("host not idle")
    if a.churn:
        for session in range(a.sessions):
            for n in [10_000,100_000]:
                for mode in ["none","cursor","many","rolling","log_storage","log_default","log_4096","mixed"]:
                    ident=f"s{session}-{n}-{mode}"
                    path=a.fixtures/f"{ident}.graph"
                    os.environ.update(SWEEP_N=str(n),SWEEP_MODE=mode,SWEEP_PATH=str(path.resolve()))
                    stem=a.out/ident
                    result=invoke(prefix+["db::shape_sweep::run_churn","--ignored","--exact","--nocapture"],
                                  stem,3600,memory_cap,disk_floor,a.fixtures)
                    result.update(case=ident)
                    rows=checked_lines(stem.with_suffix(".out"))
                    if result["exit_status"]==0:
                        if (len([r for r in rows if r["kind"]=="generation"])!=544
                                or len([r for r in rows if r["kind"]=="release"])!=1
                                or len([r for r in rows if r["kind"]=="completed"])!=1
                                or not all(r.get("verified") for r in rows)):
                            raise ValueError(f"incomplete generation evidence: {ident}")
                        with (a.out/"generations.jsonl").open("a") as f:
                            for row in rows:
                                row.update(session=session,case=ident)
                                f.write(json.dumps(row)+"\n")
                        # Scratch database only; retain all raw generation evidence.
                        path.unlink()
                    records.append(result)
                    (a.out/"dispositions.json").write_text(json.dumps(records,indent=2))
                    print(ident,result["disposition"],flush=True)
    else:
        cases=json.loads((a.public_results/"cases.json").read_text())
        public=[json.loads(l) for l in (a.public_results/"samples.jsonl").read_text().splitlines()]
        successful={r["case"]:r for r in public if r["disposition"]=="measured"}
        for i,c in enumerate(cases):
            if c["id"] not in successful:
                continue
            sample=successful[c["id"]]
            path=a.fixtures/(sample["fixture"]+".graph")
            os.environ.update(SWEEP_PATH=str(path.resolve()),SWEEP_QUERY=c["query"],SWEEP_CACHE=str(c["cache"]),
                              SWEEP_RECURSION="yes" if c["family"]=="recursion" else "no")
            stem=a.out/f"case-{i:03}"
            result=invoke(prefix+["db::shape_sweep::query_memory","--ignored","--exact","--nocapture"],
                          stem,600,memory_cap,disk_floor,a.fixtures)
            rows=checked_lines(stem.with_suffix(".out"))
            result.update(case=c["id"],observations=rows)
            if result["exit_status"]==0:
                mem=[r for r in rows if r["kind"]=="memory"]
                if len(mem)!=1 or mem[0]["rows"]!=sample["rows"] or mem[0]["meter_after_drop"]!=0:
                    raise ValueError("diagnostic answer count/release differs from public run")
            records.append(result)
            (a.out/"observations.json").write_text(json.dumps(records,indent=2))
            print(c["id"],result["disposition"],flush=True)


if __name__=="__main__":
    main()
