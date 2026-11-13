#!/usr/bin/env python3
"""V-2: classify every test-introducing site in the workspace by whether the
placement attribute can be written on it. Text-level scan (no rustc): attributes
are read from the lines directly above a `fn`; `proptest!` bodies, `macro_rules!`
bodies and non-unit return types are tracked by brace depth. Read-only."""
import re, os, sys, json, collections
ROOTS = ['crates', 'xtask', 'tests']
TEST_ATTR = re.compile(r'^\s*#\[\s*(tokio::test|test|quint_test|quint_run|rstest|test_case|async_std::test|test_log::test|tracing_test::traced_test|[\w:]*::test)\b')
FN = re.compile(r'^\s*(?:pub(?:\([^)]*\))?\s+)?(?:async\s+)?(?:unsafe\s+)?(?:const\s+)?fn\s+(\w+)\s*(?:<[^>]*>)?\s*\(')

def files():
    for root in ROOTS:
        for d, dirs, fs in os.walk(root):
            dirs[:] = [x for x in dirs if x not in ('target', 'node_modules', '.git')]
            for f in fs:
                if f.endswith('.rs'):
                    yield os.path.join(d, f)

def scan(path):
    lines = open(path, errors='replace').read().split('\n')
    out = []
    attrs = []          # pending attribute lines
    stack = []          # context stack: (kind, depth_at_open)
    depth = 0
    pending_ctx = None
    i = 0
    in_block_comment = False
    while i < len(lines):
        raw = lines[i]
        line = raw
        stripped = line.strip()
        if stripped.startswith('//'):
            i += 1
            continue
        # context openers
        m = re.match(r'^\s*(proptest|macro_rules!?)\b', stripped)
        if re.match(r'^\s*(?:\w+::)*proptest!\s*[\{\(]', stripped):
            pending_ctx = 'proptest'
        elif re.match(r'^\s*macro_rules!\s*\w+\s*\{', stripped):
            pending_ctx = 'macro_rules'
        if stripped.startswith('#['):
            # accumulate possibly multi-line attribute
            buf = stripped
            while buf.count('[') > buf.count(']') and i + 1 < len(lines):
                i += 1
                buf += ' ' + lines[i].strip()
            attrs.append(buf)
            i += 1
            continue
        fm = FN.match(line)
        if fm:
            # full signature to the opening brace for the return type
            sig = line
            j = i
            while '{' not in sig and ';' not in sig and j + 1 < len(lines):
                j += 1
                sig += ' ' + lines[j].strip()
            test_attrs = [a for a in attrs if TEST_ATTR.match(a)]
            ret = re.search(r'\)\s*->\s*([^{;]+)', sig)
            ctx = stack[-1][0] if stack else 'item'
            ctxs = [k for k, _ in stack]
            if test_attrs or ctx == 'proptest':
                out.append(dict(file=path, line=i + 1, name=fm.group(1), attrs=[a[:60] for a in attrs],
                                test_attrs=[TEST_ATTR.match(a).group(1) for a in test_attrs],
                                ret=ret.group(1).strip() if ret else None,
                                ctx='proptest' if 'proptest' in ctxs else ('macro_rules' if 'macro_rules' in ctxs else 'item'),
                                has_params=bool(re.search(r'fn\s+\w+\s*(?:<[^>]*>)?\s*\(\s*\S', sig.split('{')[0]) and not re.search(r'fn\s+\w+\s*(?:<[^>]*>)?\s*\(\s*\)', sig))))
            attrs = []
        elif stripped and not stripped.startswith('#!['):
            if not stripped.startswith('///') and not stripped.startswith('//!'):
                attrs = [] if not re.match(r'^(pub\s+)?(mod|use|struct|enum|impl|const|static|type|trait|extern)\b', stripped) else []
        # brace tracking (strings/chars ignored coarsely)
        code = re.sub(r'"(?:\\.|[^"\\])*"', '""', line)
        code = re.sub(r"'(?:\\.|[^'\\])'", "''", code)
        code = code.split('//')[0]
        for ch in code:
            if ch == '{':
                depth += 1
                if pending_ctx:
                    stack.append((pending_ctx, depth)); pending_ctx = None
            elif ch == '}':
                if stack and stack[-1][1] == depth:
                    stack.pop()
                depth -= 1
        i += 1
    return out

def in_integration_scope(p):
    return ('/tests/' in p) or p.startswith('tests/')

if __name__ == '__main__':
    allsites = []
    for p in files():
        allsites += scan(p)
    c = collections.Counter()
    problems = collections.defaultdict(list)
    for s in allsites:
        kinds = s['test_attrs'] or ['(proptest-inner)']
        key = ('proptest-inner' if s['ctx'] == 'proptest' else 'macro_rules-template' if s['ctx'] == 'macro_rules' else '/'.join(kinds))
        c[key] += 1
        if s['ret'] and s['ret'] not in ('()',) and 'quint' not in key:
            problems['non-unit return'].append(s)
        if s['has_params'] and s['ctx'] != 'proptest':
            problems['has parameters'].append(s)
        if s['ctx'] == 'macro_rules':
            problems['macro_rules template'].append(s)
        if any(a for a in s['test_attrs'] if a in ('rstest', 'test_case')):
            problems['parametrised generator'].append(s)
    print('total test-introducing sites:', len(allsites))
    for k, v in c.most_common():
        print(f'  {v:5d}  {k}')
    print('sites in */tests/* or tests/*:', sum(1 for s in allsites if in_integration_scope(s['file'])))
    print('sites under tests/integration/:', sum(1 for s in allsites if '/tests/integration/' in s['file']))
    print('\n# sites that need more than the plain attribute position (candidates for "cannot take")')
    for k, v in problems.items():
        print(f'{k}: {len(v)}')
        for s in v[:60]:
            print(f"    {s['file']}:{s['line']}  fn {s['name']}  attrs={s['test_attrs']} ret={s['ret']} ctx={s['ctx']}")
    # tests generated by macro_rules! templates carrying a test attribute
    print('\n# macro_rules! templates that carry a test attribute')
    tot = 0
    for p in files():
        src = open(p, errors='replace').read()
        for m in re.finditer(r'macro_rules!\s*(\w+)\s*\{', src):
            i = m.end(); depth = 1
            while i < len(src) and depth:
                depth += (src[i] == '{') - (src[i] == '}'); i += 1
            body = src[m.end():i]
            n = len(re.findall(r'#\[\s*(?:tokio::test|test|quint_test|quint_run)\b', body))
            if n:
                inv = re.search(r'\b%s!\s*\{(.*?)\n\}' % m.group(1), src[i:], re.S)
                gen = len(re.findall(r'=>', inv.group(1))) if inv else -1
                print(f'  {p}: macro {m.group(1)}: {n} template site(s), generates {gen} tests')
                tot += gen
    # guest-placed tests (table kinds other than host-pure) by the attribute form of their source site
    table = json.load(open('infra/metal/test-placement.json'))['placements']
    by_name = collections.defaultdict(list)
    for s_ in allsites:
        by_name[s_['name']].append(s_)
    form = collections.Counter(); unmatched = []
    placed = 0
    for b, ts in table.items():
        for t, d in ts.items():
            if d['kind'] == 'host-pure':
                continue
            placed += 1
            sites = by_name.get(t.rsplit('::', 1)[-1], [])
            if not sites:
                unmatched.append((b, t)); continue
            f = {('proptest' if x['ctx'] == 'proptest' else '/'.join(x['test_attrs'])) for x in sites}
            form['|'.join(sorted(f))] += 1
    print(f'\n# non-host-pure table entries: {placed}; source form of their fn (by fn name)')
    for k, v in form.most_common():
        print(f'  {v:4d}  {k}')
    print(f'  unmatched by fn name ({len(unmatched)}): macro-generated or defined in a binary built from a bin/ source:')
    for b, t in unmatched[:20]:
        print(f'     {b}  {t}')
    json.dump(allsites, open(sys.argv[1] if len(sys.argv) > 1 else '/dev/null', 'w'))
