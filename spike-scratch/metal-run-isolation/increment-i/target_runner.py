import os,sys
from pathlib import Path

wrapper=sys.argv[1];binary=sys.argv[2];arguments=sys.argv[3:]
identity=os.environ.get('NEXTEST_TEST_NAME')
if identity is None:
    if '--list' not in arguments:
        print('METAL-DESIGN-RUNNER-ERROR error=unknown_invocation_context',file=sys.stderr)
        raise SystemExit(78)
    os.execvpe(binary,[binary,*arguments],os.environ)
if os.environ.get('NEXTEST_BINARY_ID')=='overdrive-sim':
    print('METAL-DESIGN-PURE-HOST namespace='+os.readlink('/proc/self/ns/net')+' identity='+identity,flush=True)
    os.execvpe(binary,[binary,*arguments],os.environ)
helper=Path(__file__).resolve().parent.parent/'increment-g/local_netns_exec.py'
os.execvpe('python3',['python3',str(helper),wrapper,binary,*arguments],os.environ)
