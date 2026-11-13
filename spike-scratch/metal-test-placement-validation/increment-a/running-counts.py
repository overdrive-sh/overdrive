#!/usr/bin/env python3
"""For each test in a nextest `--success-output immediate` log, print the libtest
`running N test(s)` count its `--exact` invocation reported (2 = duplicate #[test])."""
import re, sys
for path in sys.argv[1:]:
    print('##', path.split('/')[-1])
    cur = None
    for line in open(path, errors='replace'):
        m = re.match(r'\s+(PASS|FAIL)\s+\[[^\]]*\]\s+\(\s*\d+/\d+\)\s+\S+\s+(\S+::\S+|\S+)\s*$', line)
        if m:
            cur = m.group(2)
            continue
        m = re.match(r'\s+running (\d+) tests?\s*$', line)
        if m and cur:
            print(f'  running {m.group(1)}  {cur}')
            cur = None
