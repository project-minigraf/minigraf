"""Finite manual matrix. Enumeration creates no database and does no timing."""
import json


def matrix(smoke=False):
    cases = []

    def add(family, n, shape, operation, **options):
        c = dict(family=family, n=n, shape=shape, operation=operation, k=0,
                 offset=0, limit=None, cache=256, pending=False)
        c.update(options)
        c["id"] = "/".join(str(c[k]) for k in (
            "family", "n", "shape", "operation", "k", "offset", "limit", "cache", "pending"))
        cases.append(c)

    for n in ([1000] if smoke else [100_000, 1_000_000, 10_000_000]):
        for pending in [False, True]:
            for direction in ["asc", "desc"]:
                for offset, limit in [(0, 10), (n//2, 1000), (0, None)]:
                    add("scalar", n, "permuted", direction, pending=pending, offset=offset, limit=limit)
        # Exact 3N input facts; metadata never calls this a ten-million-fact fixture.
        add("compound", n//3, "ties", "page", offset=137, limit=1000)
        for shape in ["one", "hundred", "singleton", "skew"]:
            for op in ["sum", "rank"]:
                add("window", n//2, shape, op)
        for shape in ["hundred", "skew"]:
            add("window", n//2, shape, "incompatible")
        if n <= 1_000_000:
            add("window", n//2, "skew", "sum", pending=True)

    for n in ([100] if smoke else [10_000]):
        for k in ([0, 4, 32] if smoke else [0, 1, 4, 8, 16, 32, 64, 256, 1024]):
            for shape in ["repeated", "unique", "skew"]:
                for op in ["project", "membership"]:
                    add("set", n, shape, op, k=k)
                if k in [0, 4, 32, 256]:
                    add("set", n, shape, "collect", k=k)
        # Matched pending controls; wide pending Cartesian sweep stays deferred.
        for shape in ["repeated", "unique", "skew"]:
            add("set", n, shape, "membership", k=32, pending=True)
    if not smoke:
        for k in [4, 32, 256]:
            for shape in ["repeated", "unique"]:
                for op in ["project", "membership"]:
                    add("set", 100_000, shape, op, k=k)

    for n in ([100] if smoke else [10_000, 100_000]):
        for shape in ["dense", "descending", "sparse", "permuted", "hot"]:
            add("uuid", n, shape, "refs")
        for k in [1, 16, 64, 1024]:
            add("uuid", n, "repeat", "refs", k=k)
        if not smoke:
            for cache in [0, 4096]:
                for shape, k in [("dense",0), ("sparse",0), ("repeat",1024)]:
                    add("uuid", n, shape, "refs", k=k, cache=cache)
            for shape, k in [("dense",0), ("sparse",0), ("repeat",16)]:
                add("uuid", n, shape, "project", k=k)
    for shape, n in ([("chain",10),("tree",110)] if smoke else
                     [("chain",10),("chain",100),("chain",1000),("tree",1110),("tree",11110)]):
        for pending in [False, True]:
            for k in ([1] if smoke or shape=="chain" else [1,16,128,129]):
                add("recursion",n,shape,"ancestors",k=k,pending=pending)
    assert len(cases) == len({c["id"] for c in cases})
    return cases


if __name__ == "__main__":
    import sys
    print(json.dumps(matrix("--smoke" in sys.argv), indent=2))
