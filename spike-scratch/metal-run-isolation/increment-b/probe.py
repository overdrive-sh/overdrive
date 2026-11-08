import json, os, subprocess, sys, tempfile, uuid
from pathlib import Path

mode = sys.argv[1] if len(sys.argv) > 1 else '--drive'
if mode == '--child':
    root = Path(sys.argv[2])
    child = subprocess.Popen(['/usr/bin/sleep', '20'], start_new_session=True)
    (root / 'pid').write_text(str(child.pid))
    (root / 'claim').mkdir()
    sys.exit(0)
if mode == '--post':
    root = Path(sys.argv[2])
    pid = int((root / 'pid').read_text())
    try:
        os.kill(pid, 0)
        absent = False
    except ProcessLookupError:
        absent = True
    result = {'descendant_gone_before_stop_post': absent, 'claim_survived_main_exit': (root / 'claim').is_dir(), 'kernel': os.uname().release}
    (root / 'result.json').write_text(json.dumps(result, sort_keys=True))
    if absent:
        (root / 'claim').rmdir()
    sys.exit(0 if absent else 1)
script = str(Path(__file__).resolve())
unit = 'overdrive-metal-design-' + uuid.uuid4().hex[:16]
with tempfile.TemporaryDirectory(prefix='metal-isolation-stoppost-') as directory:
    try:
        outcome = subprocess.run(['systemd-run', '--quiet', '--wait', '--collect', '--unit='+unit, '--property=Type=exec', '--property=KillMode=control-group', '--property=TimeoutStopSec=5s', '--property=ExecStopPost=/usr/bin/python3 '+script+' --post '+directory, '/usr/bin/python3', script, '--child', directory], timeout=15, capture_output=True, text=True)
        result = json.loads((Path(directory) / 'result.json').read_text())
        result['systemd_run_status'] = outcome.returncode
        result['claim_released_after_children_gone'] = not (Path(directory) / 'claim').exists()
        print(json.dumps(result, sort_keys=True))
        assert outcome.returncode == 0 and all(result[k] for k in ['descendant_gone_before_stop_post', 'claim_survived_main_exit', 'claim_released_after_children_gone'])
    finally:
        subprocess.run(['systemctl', 'stop', unit], timeout=7, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
