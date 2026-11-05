"""Matched recursive pairs on separate sockets, swapping placement each round."""
import argparse
import hashlib
from concurrent.futures import ThreadPoolExecutor
import json
from pathlib import Path
import shutil
import subprocess

from cases import matrix
from evidence import summarize
from run import idle_samples, invoke, memory_caps


def copy_pair_fixtures(source, out, stems):
    """Independent inodes avoid exclusive writer-lock waits between paired runs."""
    out.mkdir()
    copies = {role: out / role for role in ["baseline", "control"]}
    for path in copies.values():
        path.mkdir()
    hashes = {}
    for stem in sorted(set(stems)):
        for suffix in [".graph", ".json", ".graph.wal"]:
            original = source / (stem + suffix)
            if suffix == ".graph.wal" and not original.exists():
                continue
            digest = hashlib.sha256(original.read_bytes()).hexdigest()
            for directory in copies.values():
                target = directory / original.name
                shutil.copyfile(original, target)
                if hashlib.sha256(target.read_bytes()).hexdigest() != digest:
                    raise ValueError("paired fixture copy differs")
            hashes[original.name] = digest
    (out / "hashes.json").write_text(json.dumps(hashes, indent=2))
    return copies


def main():
    p=argparse.ArgumentParser()
    p.add_argument("--binaries",type=Path,required=True)
    p.add_argument("--out",type=Path,required=True)
    p.add_argument("--fixtures",type=Path,required=True)
    p.add_argument("--sessions",type=int,default=3)
    p.add_argument("--samples",type=int,default=5)
    p.add_argument("--smoke",action="store_true")
    a=p.parse_args()
    a.out.mkdir(parents=True,exist_ok=False)
    a.fixtures.mkdir(parents=True,exist_ok=True)
    builds=json.loads((a.binaries/"builds.json").read_text())
    controls=[v for v in builds if v!="baseline"]
    cases=[c for c in matrix(a.smoke) if c["family"]=="recursion"]
    expanded=[]
    for i,c in enumerate(cases):
        path=a.out/f"case-{i:03}.json"
        path.write_text(json.dumps(c,indent=2))
        c["query"]=subprocess.check_output([a.binaries/"baseline","query",path,a.fixtures],text=True).strip()
        path.write_text(json.dumps(c,indent=2))
        for control in controls:
            for variant in ["baseline",control]:
                expanded.append({**c,"id":f"{c['id']}/{control}/{variant}",
                                 "variant":variant,"control":control,"original_case":c["id"]})
    (a.out/"cases.json").write_text(json.dumps(expanded,indent=2))
    (a.out/"builds.json").write_text(json.dumps(builds,indent=2))
    bounds=memory_caps(["1","3"])
    caps={node:min(cap,48*1024**3) for node,cap in zip(bounds["nodes"],bounds["child_caps_bytes"])}
    (a.out/"run.json").write_text(json.dumps({"memory_bounds":bounds,"effective_node_caps_bytes":caps,
                                             "sessions":a.sessions,"samples":a.samples},indent=2))
    disk=int(shutil.disk_usage(a.fixtures).free*0.2)
    idle=idle_samples()
    (a.out/"idle.json").write_text(json.dumps(idle))
    if any(s["idle_percent"]<95 or s["swapped_pages"] for s in idle):
        raise RuntimeError("host not idle")
    stems=[]
    for i,c in enumerate(cases):
        result=invoke([str(a.binaries/"baseline"),"build",str(a.out/f"case-{i:03}.json"),str(a.fixtures)],
                      a.out/f"build-{i:03}",3600,min(caps.values()),disk,a.fixtures)
        if result["exit_status"]:
            raise RuntimeError(f"recursive fixture failed: {result}")
        stems.append(json.loads((a.out/f"build-{i:03}.out").read_text())["fixture"])
    copies=copy_pair_fixtures(a.fixtures,a.out/"pair-fixtures",stems)
    failed={}
    rows=[]
    with (a.out/"samples.jsonl").open("w") as output, ThreadPoolExecutor(max_workers=2) as pool:
        for session in range(a.sessions):
            for i,c in enumerate(cases):
                for control in controls if session%2==0 else reversed(controls):
                    pending={}
                    results={}
                    for role,variant in enumerate(["baseline",control]):
                        ident=f"{c['id']}/{control}/{variant}"
                        node="1" if (session+role)%2==0 else "3"
                        fixture_dir=copies["baseline" if role==0 else "control"]
                        stem=a.out/f"s{session}-c{i:03}-{control}-{variant}"
                        if ident in failed:
                            results[variant]=({"disposition":"deferred","exit_status":None,
                                               "reason":str(failed[ident])},None,node)
                            continue
                        command=["numactl",f"--cpunodebind={node}",f"--membind={node}",str(a.binaries/variant),
                                 "batch",str(a.out/f"case-{i:03}.json"),str(fixture_dir),str(a.samples)]
                        future=pool.submit(invoke,command,stem,600,caps[node],disk,a.fixtures)
                        pending[variant]=(future,stem,node,ident)
                    for variant,(future,stem,node,ident) in pending.items():
                        status=future.result()
                        values=None
                        if status["disposition"]=="measured":
                            values=json.loads(stem.with_suffix(".out").read_text())
                            if not isinstance(values,list) or len(values)!=a.samples:
                                raise ValueError("wrong paired batch length")
                        else:
                            failed[ident]=status.copy()
                        results[variant]=(status,values,node)
                    for variant,(status,values,node) in results.items():
                        for sample in range(a.samples):
                            result=status.copy()
                            if values is not None:
                                result.update(values[sample])
                                if result["sample_in_process"]!=sample:
                                    raise ValueError("wrong paired sample identity")
                                wal=a.fixtures/(result["fixture"]+".graph.wal")
                                result["wal_bytes"]=wal.stat().st_size if wal.exists() else 0
                                result["total_storage_bytes"]=result["file_bytes"]+result["wal_bytes"]
                            result.update(case=f"{c['id']}/{control}/{variant}",session=session,sample=sample,
                                          node=node,variant=variant,control=control,original_case=c["id"],
                                          block=f"{session}/{i}/{control}/{sample}",memory_cap_bytes=caps[node])
                            rows.append(result)
                            output.write(json.dumps(result)+"\n");output.flush()
                print(session,c["id"],flush=True)
    (a.out/"summary.json").write_text(json.dumps(summarize(expanded,rows,a.sessions,a.samples),indent=2))
    baseline=a.out/"baseline"
    baseline.mkdir()
    selected_cases=[{**c,"id":c["original_case"]} for c in expanded
                    if c["variant"]=="baseline" and c["control"]==controls[0]]
    selected_rows=[{**r,"case":r["original_case"]} for r in rows
                   if r["variant"]=="baseline" and r["control"]==controls[0]]
    (baseline/"cases.json").write_text(json.dumps(selected_cases,indent=2))
    (baseline/"samples.jsonl").write_text("".join(json.dumps(r)+"\n" for r in selected_rows))
    (baseline/"summary.json").write_text(json.dumps(summarize(selected_cases,selected_rows,a.sessions,a.samples),indent=2))


if __name__=="__main__":
    main()
