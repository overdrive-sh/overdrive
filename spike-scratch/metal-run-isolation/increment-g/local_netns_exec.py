import ctypes, os, sys
from pathlib import Path
libc=ctypes.CDLL(None,use_errno=True)
for phase in ['parent','local']:
    if libc.unshare(0x40000000):
        raise OSError(ctypes.get_errno(),'unshare '+phase)
    if phase=='parent':
        Path('/proc/sys/net/vsock/child_ns_mode').write_text('local\n')
assert Path('/proc/sys/net/vsock/ns_mode').read_text().strip()=='local'
print('METAL-DESIGN-LOCAL-NAMESPACE',os.readlink('/proc/self/ns/net'),flush=True)
os.execvpe(sys.argv[1],sys.argv[1:],os.environ)
