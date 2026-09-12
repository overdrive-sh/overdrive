#!/usr/bin/env python3
"""Analyze retained observation logs; never drive product or write expectations."""
import datetime
import pathlib
import re
import sys

ansi = re.compile(r"\x1b\[[0-9;]*m")
for name in sys.argv[1:]:
    path = pathlib.Path(name)
    entries = []
    for number, raw in enumerate(path.read_text().splitlines(), 1):
        line = ansi.sub("", raw)
        if not re.match(r"\d{4}-", line):
            continue
        stamp = datetime.datetime.fromisoformat(line.split()[0])
        entries.append((stamp, number, line))
    allocations = sorted({re.search(r"alloc=(\S+)", row[2])[1] for row in entries
                          if "issue283 VM start enter" in row[2]})
    print(f"\nsource={path}")
    for alloc in allocations:
        def select(message, field, value, before=None):
            matches = [row for row in entries if message in row[2]
                       and re.search(rf"(?:^|\s){field}={re.escape(value)}(?:\s|$)", row[2])
                       and (before is None or row[0] <= before)]
            return (matches[-1] if before else matches[0]) if matches else None

        def duration(label, first, last):
            if first and last:
                delta = (last[0] - first[0]).total_seconds() * 1000
                print(f"  {label}: {delta:.3f} ms [lines {first[1]} -> {last[1]}]")

        print(f"allocation={alloc}")
        start = select("VM start enter", "alloc", alloc)
        prep = select("host preparation complete", "alloc", alloc)
        create = select("VMM create complete", "alloc", alloc)
        ready = select("READY accepted", "alloc", alloc)
        if not create:
            continue
        pid = re.search(r"pid=(\d+)", create[2])[1]
        workload = alloc.removeprefix("alloc-").removesuffix("-0")
        enqueue = select("handler enqueue", "workload", workload, start[0])
        evaluate = select("convergence evaluation enter reconciler=workload-lifecycle",
                          "target_name", "workload/" + workload, start[0])
        duration("enqueue to evaluation (queue)", enqueue, evaluate)
        duration("evaluation to driver entry (hydrate/persist/dispatch/network/identity)", evaluate, start)
        duration("driver host preparation", start, prep)
        duration("VMM adapter creation including clone/copy/spawn", prep, create)
        duration("VMM creation to guest READY", create, ready)
        release = select("released guest EXEC", "alloc", alloc)
        reap = select("VMM reaped", "pid", pid)
        duration("READY to EXEC release", ready, release)
        stop = select("VM stop enter", "alloc", alloc)
        write = select("SHUTDOWN write finished", "pid", pid)
        terminate = select("terminate enter", "alloc", alloc)
        cleanup = select("cleanup enter", "alloc", alloc)
        cleaned = select("cleanup complete", "alloc", alloc)
        if stop and terminate:
            stop_enqueue = select("handler enqueue", "workload", workload, stop[0])
            duration("stop enqueue to driver entry", stop_enqueue, stop)
            duration("stop entry to successful SHUTDOWN write", stop, write)
            duration("stop entry to VMM terminate entry (request window)", stop, terminate)
            duration("terminate entry to reap", terminate, reap)
            duration("terminate entry to completion (grace plus reap)", terminate, cleanup)
            duration("owned artifact cleanup calls", cleanup, cleaned)
            duration("whole driver stop", stop, cleaned)
        elif reap:
            duration("EXEC release to VMM reap (no successful Live stop)", release, reap)
        if reap:
            print(f"  ending: {reap[2]}")
