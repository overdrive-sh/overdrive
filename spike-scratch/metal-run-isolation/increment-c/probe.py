import base64, hashlib, json, subprocess, tempfile
from pathlib import Path

def git(root, *args):
    return subprocess.check_output(['git', '-C', str(root), *args], stderr=subprocess.DEVNULL).decode().strip()

def identify(root):
    checkout = Path(git(root, 'rev-parse', '--show-toplevel')).resolve()
    own = Path(git(root, 'rev-parse', '--absolute-git-dir')).resolve()
    common = Path(git(root, 'rev-parse', '--path-format=absolute', '--git-common-dir')).resolve()
    if own != common:
        kind, raw = 'worktree', str(own).encode()
    else:
        kind = 'branch'
        raw = str(checkout).encode() + b'\0' + git(root, 'symbolic-ref', '--quiet', 'HEAD').encode()
    key = hashlib.sha256(b'overdrive-metal-checkout-v1\0'+kind.encode()+b'\0'+raw).hexdigest()
    return kind, key, base64.b64encode(raw).decode()

with tempfile.TemporaryDirectory(prefix='metal-identity-design-') as tmp:
    root = Path(tmp)/'main'
    root.mkdir()
    git(root, 'init', '--initial-branch=metal/isolation')
    git(root, 'commit', '--allow-empty', '-m', 'Disposable identity fixture\n\nCo-Authored-By: Codex <codex@openai.com>')
    a,b = Path(tmp)/'same-a',Path(tmp)/'same-b'
    git(root, 'worktree', 'add', '--force', str(a), 'metal/isolation')
    git(root, 'worktree', 'add', '--force', str(b), 'metal/isolation')
    before_a, before_b = identify(a), identify(b)
    assert before_a[0] == before_b[0] == 'worktree' and before_a[1] != before_b[1]
    git(a, 'checkout', '-b', 'metal-isolation')
    assert identify(a) == before_a
    main_before = identify(root)
    assert main_before[0] == 'branch'
    git(root, 'checkout', '--detach')
    try:
        identify(root)
        raise AssertionError('detached ordinary clone must refuse')
    except subprocess.CalledProcessError:
        pass
    git(a, 'checkout', '--detach')
    assert identify(a) == before_a
    alias = Path(tmp)/'alias'
    alias.symlink_to(a, target_is_directory=True)
    assert identify(alias) == before_a
    print(json.dumps({'linked_same_branch_distinct':True,'linked_branch_switch_stable':True,'detached_linked_stable':True,'ordinary_clone_branch_fallback':True,'detached_ordinary_refused':True,'symlink_alias_same_identity':True},sort_keys=True))
