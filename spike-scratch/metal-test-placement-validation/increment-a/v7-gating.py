#!/usr/bin/env python3
"""V-7: for each guest-placed (non-host-pure) test in infra/metal/test-placement.json, which Cargo
features must be enabled for the test to COMPILE (so which off-Metal lanes even build it)?
Needs /tmp/v2-sites.json first:  python3 v2-count.py /tmp/v2-sites.json
Text-level: reads `#![cfg(feature = ..)]` of the test's file and of every ancestor entry file,
`#[cfg(feature = ..)]` on the `mod x;` declarations leading to it, and on the fn."""
import json, re, os, collections, sys
sites = json.load(open('/tmp/v2-sites.json'))
by_name = collections.defaultdict(list)
for s in sites: by_name[s['name']].append(s)
table = json.load(open('infra/metal/test-placement.json'))['placements']
FEAT = re.compile(r'feature\s*=\s*"([\w-]+)"')


def attr_block_features(lines, decl_index):
    """cfg features in the attribute block directly above lines[decl_index] (multi-line attrs included)."""
    feats = set(); j = decl_index - 1
    while j >= 0:
        t = lines[j].strip()
        if not t or t.endswith(';') or t.endswith('}') or t.endswith('{'):
            break
        if t.startswith('///') or t.startswith('//'):
            j -= 1; continue
        if t.startswith('#[cfg'):
            feats |= set(FEAT.findall(t))
        j -= 1
    return feats

def file_cfg(path):
    out = set()
    try: src = open(path, errors='replace').read()
    except OSError: return out
    for m in re.finditer(r'^\s*#!\[cfg\((.*?)\)\]\s*$', src, re.M):
        out |= set(FEAT.findall(m.group(1)))
    return out

def mod_decl_cfg(parent, mod):
    out = set()
    try: lines = open(parent, errors='replace').read().split('\n')
    except OSError: return out
    for i, l in enumerate(lines):
        if re.match(r'\s*(pub\s+)?mod\s+%s\s*[;{]' % re.escape(mod), l):
            out |= attr_block_features(lines, i)
    return out

def chain_features(path):
    """features needed to compile `path`, following tests/<entry>.rs -> tests/<entry>/mod.rs ancestry"""
    feats = file_cfg(path)
    d = os.path.dirname(path); base = os.path.basename(path)[:-3]
    # integration-style: tests/integration/<m>.rs, entry tests/integration.rs
    parts = path.split('/')
    if 'tests' in parts:
        ti = parts.index('tests')
        if len(parts) > ti + 2:                       # tests/<entry>/.../<m>.rs
            entry = '/'.join(parts[:ti + 1] + [parts[ti + 1] + '.rs'])
            feats |= file_cfg(entry)
            chain = parts[ti + 2:]
            parent = entry
            for seg in chain:
                m = seg[:-3] if seg.endswith('.rs') else seg
                feats |= mod_decl_cfg(parent, m)
                nxt = '/'.join(parts[:ti + 2] + chain[:chain.index(seg) + 1])
                parent = nxt if nxt.endswith('.rs') else nxt + '/mod.rs'
    else:                                              # src file: its parent chain
        cur = path
        for _ in range(8):
            d = os.path.dirname(cur); b = os.path.basename(cur)[:-3]
            for cand in (os.path.join(d, 'mod.rs'), d + '.rs', os.path.join(d, 'lib.rs')):
                if os.path.exists(cand) and cand != cur:
                    feats |= mod_decl_cfg(cand, b if b != 'mod' else os.path.basename(d)); cur = cand; break
            else: break
    return feats

def inline_mod_cfg(path, line):
    """features from `#[cfg(..)]` on every inline `mod x {` enclosing `line` in the same file"""
    try: lines = open(path, errors='replace').read().split('\n')
    except OSError: return set()
    out = set(); stack = []; depth = 0
    for i, l in enumerate(lines[:line], 1):
        code = re.sub(r'"(?:\\.|[^"\\])*"', '""', l).split('//')[0]
        m = re.match(r'\s*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*\{', code)
        if m:
            stack.append((depth + 1, attr_block_features(lines, i - 1)))
        for ch in code:
            if ch == '{': depth += 1
            elif ch == '}':
                if stack and stack[-1][0] == depth: stack.pop()
                depth -= 1
    for _, f in stack: out |= f
    return out

rows = collections.Counter(); detail = collections.defaultdict(list); unmatched = []
for b, ts in table.items():
    for t, d in ts.items():
        if d['kind'] == 'host-pure': continue
        name = t.rsplit('::', 1)[-1]
        cand = by_name.get(name, [])
        if not cand: unmatched.append((b, t, d['kind'])); continue
        # choose the site whose file belongs to the binary's crate
        crate = b.split('::')[0]
        cand2 = [s for s in cand if ('crates/%s/' % crate in s['file'] or ('tests/conformance' in s['file'] and 'conformance' in crate) or s['file'].startswith(crate))] or cand
        s = cand2[0]
        feats = chain_features(s['file']) | inline_mod_cfg(s['file'], s['line']) | set(FEAT.findall(' '.join(a for a in s['attrs'] if 'cfg' in a)))
        key = (b, d['kind'], tuple(sorted(feats)))
        rows[key] += 1
        detail[key].append(t)

print('# guest-placed tests by binary and the features needed to compile them (text-level)')
by_feat = collections.Counter()
for (b, k, f), n in sorted(rows.items()):
    print(f'{n:4d}  {b:62s} {k:22s} needs: {",".join(f) or "(nothing: default features)"}')
    by_feat[f] += n
print('\n# totals by feature set')
tot = 0
for f, n in sorted(by_feat.items(), key=lambda x: -x[1]):
    print(f'{n:4d}  {",".join(f) or "(default features)"}'); tot += n
print('matched:', tot, ' unmatched:', len(unmatched))
for u in unmatched: print('   unmatched', u)
