#!/usr/bin/env python3
"""V-3: derive, from guest_suite.rs (read as text) and infra/metal/test-placement.json,
the per-test guest environment variants of the nextest per-test path
(`nextest_guest_plan` and its callees) and assign every guest-group test to one.
Read-only analysis; writes nothing but stdout."""
import json, re, sys, collections
SRC = open('crates/overdrive-testing/src/guest_suite.rs').read()
TABLE = json.load(open('infra/metal/test-placement.json'))['placements']

def consts():
    out = {}
    for m in re.finditer(r'^const (\w+): &str =\s*\n?\s*"([^"]*)";', SRC, re.M):
        out[m.group(1)] = m.group(2)
    return out
STR = consts()

def cells(name):
    m = re.search(r'^const %s: &\[Cell\] = &\[(.*?)\];' % name, SRC, re.M | re.S)
    body = m.group(1)
    res = []
    for e in re.finditer(r'\(\s*("([^"]*)"|(\w+))\s*,\s*(\d+)\s*,\s*(true|false)\s*,?\s*\)', body):
        s = e.group(2) if e.group(2) is not None else STR[e.group(3)]
        res.append((s, int(e.group(4)), e.group(5) == 'true'))
    return res

TABLES = {n: cells(n) for n in [
    'STEP13_04_CONTROL_PLANE', 'STEP13_04_WORKER', 'STEP13_04_HOST_LOCAL_FIREWALL',
    'STEP13_04_REMOTE_FIREWALL', 'STEP13_05_CONTROL_PLANE', 'STEP13_05_WORKER',
    'STEP13_06_WORKER', 'STEP13_06_ROUTED_WORKER', 'CONTROL_PLANE_NORMAL_NATIVE',
    'STEP14_01_CONTROL_PLANE', 'STEP14_01_CLI', 'STEP14_02_CLI', 'STEP14_02_WORKER',
    'VM_COHORT_CLI', 'NESTED_GUEST_LAUNCH', 'NESTED_GUEST_RUSTC']}

def sel(table, test):
    return any((test == f) if exact else test.startswith(f) for f, _, exact in TABLES[table])

MTLS_RELAY = STR['MTLS_RELAY']
DECLARED = re.search(r'const DECLARED_LISTENER_TCP_CAPTURE_TEST: &str = "([^"]*)"', SRC).group(1)
LANE_DRIVERS = re.search(r'const IN_GUEST_LANE_DRIVERS: &str = "([^"]*)"', SRC).group(1)

def variant(group, test):
    """The environment the dispatcher builds for (group, test) on the nextest path."""
    if group != 'cgroup-default':
        if group == 'mtls-relay':
            return 'mtls-relay/relay' if test.startswith(MTLS_RELAY) else 'mtls-relay/native'
        return group
    focused = any(sel(t, test) for t in ('STEP14_01_CONTROL_PLANE', 'STEP14_01_CLI', 'STEP14_02_CLI', 'STEP14_02_WORKER'))
    cohort = sel('VM_COHORT_CLI', test)
    launch = sel('NESTED_GUEST_LAUNCH', test)
    if cohort:
        return 'cgroup-default/nested-cohort'
    if launch:
        if sel('NESTED_GUEST_RUSTC', test):
            return 'cgroup-default/nested-launch-rustc'
        if test.startswith(LANE_DRIVERS):
            return 'cgroup-default/nested-launch-lane'
        return 'cgroup-default/nested-launch'
    if focused:
        return 'cgroup-default/nested'
    parts = []
    if sel('CONTROL_PLANE_NORMAL_NATIVE', test): parts.append('normal-gate')
    if any(sel(t, test) for t in ('STEP13_04_CONTROL_PLANE', 'STEP13_04_WORKER', 'STEP13_04_HOST_LOCAL_FIREWALL', 'STEP13_04_REMOTE_FIREWALL')):
        parts.append('dummy-module')
        if sel('STEP13_04_HOST_LOCAL_FIREWALL', test): parts.append('host-route')
    if any(sel(t, test) for t in ('STEP13_05_CONTROL_PLANE', 'STEP13_05_WORKER')):
        parts.append('legb-tools')
        if sel('STEP13_05_WORKER', test): parts.append('legb-setup-audit')
    if sel('STEP13_06_ROUTED_WORKER', test): parts.append('routed-route')
    if test == DECLARED: parts.append('declared-listener-diagnostic')
    return 'cgroup-default' + ('/' + '+'.join(parts) if parts else '')

if __name__ == '__main__':
    print('# cell tables (name: entries, bodies)')
    for n, c in TABLES.items():
        print(f'{n}: entries={len(c)} bodies={sum(b for _, b, _ in c)} exact={sum(1 for *_, e in c if e)} prefix={sum(1 for *_, e in c if not e)}')
    by = collections.defaultdict(list)
    unmatched_cells = []
    for b, tests in TABLE.items():
        for t, d in tests.items():
            if d['kind'] == 'guest-group':
                by[variant(d['group'], t)].append((b, t))
    print('\n# guest-group tests per derived environment (total %d)' % sum(len(v) for v in by.values()))
    for k in sorted(by):
        bins = collections.Counter(b for b, _ in by[k])
        print(f'{len(by[k]):4d}  {k}   binaries: ' + ', '.join(f'{b}={n}' for b, n in sorted(bins.items())))
    # every cell must name a real placed test (no silently dead variant row)
    placed = {(t) for b, ts in TABLE.items() for t, d in ts.items() if d['kind'] == 'guest-group'}
    print('\n# cell entries that match NO placed guest-group test (dead variant rows):')
    for n, c in TABLES.items():
        for f, _, exact in c:
            hit = [t for t in placed if (t == f if exact else t.startswith(f))]
            if not hit:
                print(f'  {n}: {f}  ({"exact" if exact else "prefix"})')
    print('\n# derived environments with listed tests (cgroup-default variants other than the base)')
    for k in sorted(by):
        if k.startswith('cgroup-default/') and len(by[k]) <= 12:
            for b, t in by[k]:
                print(f'  {k}\t{b}\t{t}')
