"""One-shot diagnostic: real Quint ITF vs real public quint-connect runner.

No Metal/systemd owner is driven. This is codec evidence, not conformance.
Build codec-probe.rs as main.rs using codec-probe.Cargo.toml, then pass the binary.
"""
from pathlib import Path
import collections
import json
import os
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[5]
ART = Path(__file__).resolve().parent
SPEC = ROOT / 'specs/quint/metal-run-isolation/runner_protocol.qnt'
MANIFEST = SPEC.with_name('checks.toml')
PROBE = Path(sys.argv[1]).resolve()
SEED = '20261108'
KEY = 'runner_conf::runner_protocol::conf_step'
ENV = dict(os.environ)
ENV['PATH'] = '/Users/marcus/.npm/_npx/12f965bf8c42624e/node_modules/.bin:' + ENV['PATH']
commands = []

def call(argv, log):
    with (ART / log).open('w') as f:
        r = subprocess.run(list(map(str, argv)), env=ENV, stdout=f, stderr=subprocess.STDOUT)
    commands.append({'argv': list(map(str, argv)), 'log': log, 'exit': r.returncode})
    assert r.returncode == 0, (argv, r.returncode)

def decoded(trace):
    return [{'action': s[KEY]['tag'],
             'actor': int(s[KEY]['value']['actor']['#bigint']),
             'value': int(s[KEY]['value']['value']['#bigint'])}
            for s in trace['states']]

def split(steps):
    traces = []
    for s in steps:
        if s['action'] == 'init':
            traces.append([])
        traces[-1].append(s)
    return sorted(json.dumps(t, sort_keys=True) for t in traces)

checks = tomllib.loads(MANIFEST.read_text())['check']
named = [c['run'] for c in checks if 'run' in c]
assert len(checks) == 18 and len(named) == 13
results = []
for name in named:
    pattern = ART / f'named-{name}-{{seq}}.itf.json'
    call(['quint', 'test', SPEC, '--main', 'runner_conf', '--match', f'^{name}$',
          '--max-samples', '1', '--seed', SEED, '--out-itf', pattern, '--verbosity', '0'],
         f'named-{name}-generation.log')
    report = ART / f'named-{name}-driver.json'
    call([PROBE, SPEC, 'named:' + name, report, SEED], f'named-{name}-runner.log')
    raw = json.loads((ART / f'named-{name}-0.itf.json').read_text())
    actual = json.loads(report.read_text())
    assert actual['ok'] and actual['codec_only'] and not actual['state_comparison']
    assert actual['steps_reaching_driver'] == decoded(raw)
    assert all(not v.startswith('mbt::') for v in raw['vars'])
    results.append({'run': name, 'states': len(raw['states']), 'exact_picks_match': True})

call(['quint', 'run', SPEC, '--main', 'runner_conf', '--seed', SEED,
      '--max-samples', '4', '--n-traces', '4', '--max-steps', '32', '--out-itf',
      ART / 'random-{seq}.itf.json', '--mbt', '--verbosity', '0'], 'random-generation.log')
call([PROBE, SPEC, 'random', ART / 'random-driver.json', SEED], 'random-runner.log')
raw_traces = [json.loads(p.read_text()) for p in sorted(ART.glob('random-*.itf.json'))]
assert len(raw_traces) == 4
actual = json.loads((ART / 'random-driver.json').read_text())
assert actual['ok'] and not actual['state_comparison']
assert split(actual['steps_reaching_driver']) == sorted(json.dumps(decoded(t), sort_keys=True) for t in raw_traces)
idle = []
for t, raw in enumerate(raw_traces):
    for i, state in enumerate(raw['states']):
        assert state['mbt::actionTaken'] == state[KEY]['tag']
        if state[KEY]['tag'] == 'Idle':
            assert state[KEY]['value'] == {'actor': {'#bigint':'0'}, 'value': {'#bigint':'0'}}
            idle.append({'trace': t, 'index': i, 'mbt_action': state.get('mbt::actionTaken'),
                         'previous': raw['states'][i-1][KEY]['tag'] if i else None})
assert any(i['previous'] != 'Idle' and i['previous'] is not None for i in idle)
(ART / 'codec-results.json').write_text(json.dumps({
    'codec_only': True, 'production_conformance_claimed': False,
    'quint_connect': '0.1.2', 'seed': SEED, 'named_runs': results,
    'named_state_count': sum(r['states'] for r in results),
    'random_traces': len(raw_traces), 'random_states': len(actual['steps_reaching_driver']),
    'random_max_steps': 32, 'idle_steps': idle,
    'random_exact_picks_match': True,
}, indent=2) + '\n')
(ART / 'commands.json').write_text(json.dumps(commands, indent=2) + '\n')
print(json.dumps({'named_runs':len(named), 'named_states':sum(r['states'] for r in results),
                  'random_states':len(actual['steps_reaching_driver']), 'idle_steps':len(idle)}))
