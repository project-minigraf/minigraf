"""Regenerate paired ratios and sustained-pin envelopes from retained observations."""
import argparse
import collections
import json
from pathlib import Path
import statistics


def churn(rows, sessions=3):
    groups=collections.defaultdict(list)
    for r in rows:
        groups[r["case"]].append(r)
    wanted={f"s{s}-{n}-{m}" for s in range(sessions) for n in [10_000,100_000]
            for m in ["none","cursor","many","rolling","log_storage","log_default","log_4096","mixed"]}
    if set(groups)!=wanted:
        raise ValueError("missing/extra churn cases")
    summary=[]
    for key, group in sorted(groups.items()):
        gen=[r for r in group if r["kind"]=="generation"]
        release=[r for r in group if r["kind"]=="release"]
        done=[r for r in group if r["kind"]=="completed"]
        if (len(gen)!=544 or {r["round"] for r in gen}!=set(range(544))
                or len(release)!=1 or len(done)!=1 or not all(r.get("verified") for r in group)):
            raise ValueError(f"incomplete or unverified generations: {key}")
        gen.sort(key=lambda r:r["round"])
        if release[0]["quarantine_pages"]!=0 or any(r["quarantine_pages"] for r in gen[288:]):
            raise ValueError(f"quarantine not reclaimed: {key}")
        if release[0]["released_pages"] and done[0]["reused_pages"]<=0:
            raise ValueError(f"no released-page reuse: {key}")
        if any(b["generation"]<=a["generation"] for a,b in zip(gen,gen[1:])):
            raise ValueError("checkpoint generation did not advance")
        phases={}
        for name in ["baseline","held","after_release"]:
            part=[r for r in gen if r["phase"]==name]
            phases[name]={"generations":len(part),
                          "checkpoint_median_ms":statistics.median(r["checkpoint_ns"]/1e6 for r in part),
                          "checkpoint_max_ms":max(r["checkpoint_ns"]/1e6 for r in part),
                          "file_growth_bytes":part[-1]["file_bytes"]-part[0]["file_bytes"]}
        summary.append({"case":key,"phases":phases,
                        "peak_quarantine_pages":max(r["quarantine_pages"] for r in gen),
                        "peak_quarantine_vector_capacity_bytes":max(r["quarantine_vector_capacity_bytes"] for r in gen),
                        "peak_file_bytes":max(r["file_bytes"] for r in gen),
                        "peak_rss_kib":max(r["rss"]["peak_kib"] for r in gen),
                        "released_pages":release[0]["released_pages"],"reused_pages":done[0]["reused_pages"]})
    return summary


def ratios(rows):
    blocks=collections.defaultdict(dict)
    for r in rows:
        if r["disposition"]=="measured":
            if r["variant"] in blocks[r["block"]]:
                raise ValueError("duplicate paired variant")
            blocks[r["block"]][r["variant"]]=r
    groups=collections.defaultdict(list)
    sessions=collections.defaultdict(lambda: collections.defaultdict(list))
    placements=collections.defaultdict(lambda: collections.defaultdict(list))
    for block in blocks.values():
        baseline=block.get("baseline")
        if baseline is None:
            continue
        for variant,row in block.items():
            if variant=="baseline":
                continue
            if row["digest"]!=baseline["digest"]:
                raise ValueError("matched variants disagree on answer")
            if row["session"]!=baseline["session"]:
                raise ValueError("matched variants disagree on session")
            name=baseline.get("original_case",baseline["case"].removesuffix("/baseline"))
            key=(name,variant)
            ratio=row["elapsed_ns"]/baseline["elapsed_ns"]
            placement=(baseline["node"],row["node"])
            groups[key].append(ratio)
            sessions[key][(row["session"],*placement)].append(ratio)
            placements[key][placement].append(ratio)

    def stats(xs):
        return {"pairs":len(xs),"median_ratio_to_baseline":statistics.median(xs),
                "min_ratio":min(xs),"max_ratio":max(xs)}

    return [{"case":name,"variant":variant,**stats(xs),
             "sessions":[{"session":s,"baseline_node":b,"control_node":c,**stats(values)}
                         for (s,b,c),values in sorted(sessions[(name,variant)].items())],
             "placements":[{"baseline_node":b,"control_node":c,**stats(values)}
                           for (b,c),values in sorted(placements[(name,variant)].items())]}
            for (name,variant),xs in sorted(groups.items())]


if __name__=="__main__":
    p=argparse.ArgumentParser()
    p.add_argument("kind",choices=["churn","ratios"])
    p.add_argument("input",type=Path)
    p.add_argument("--sessions",type=int,default=3)
    a=p.parse_args()
    rows=[json.loads(l) for l in a.input.read_text().splitlines()]
    print(json.dumps(churn(rows,a.sessions) if a.kind=="churn" else ratios(rows),indent=2))
