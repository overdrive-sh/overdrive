import ctypes, errno, fcntl, json, multiprocessing, os, socket, struct
from pathlib import Path

NEWNET=0x40000000
SET_CID=0x4008af60
CID=0x01002007
PORT=1234
libc=ctypes.CDLL(None,use_errno=True)

def unshare():
    if libc.unshare(NEWNET):
        raise OSError(ctypes.get_errno(),'unshare(CLONE_NEWNET)')

def child(pipe):
    unshare()
    Path('/proc/sys/net/vsock/child_ns_mode').write_text('local\n')
    unshare()
    mode=Path('/proc/sys/net/vsock/ns_mode').read_text().strip()
    namespace=os.readlink('/proc/self/ns/net')
    fd=os.open('/dev/vhost-vsock',os.O_RDWR|os.O_CLOEXEC)
    fcntl.ioctl(fd,SET_CID,struct.pack('Q',CID))
    duplicate=os.open('/dev/vhost-vsock',os.O_RDWR|os.O_CLOEXEC)
    try:
        fcntl.ioctl(duplicate,SET_CID,struct.pack('Q',CID))
        duplicate_refused=False
    except OSError as error:
        duplicate_refused=error.errno==errno.EADDRINUSE
    finally:
        os.close(duplicate)
    listener=socket.socket(socket.AF_VSOCK,socket.SOCK_STREAM)
    listener.bind((socket.VMADDR_CID_ANY,PORT))
    listener.listen(1)
    pipe.send({'namespace':namespace,'mode':mode,'cid':CID,'port':listener.getsockname()[1],'same_namespace_duplicate_refused':duplicate_refused})
    while True:
        command=pipe.recv()
        if command=='observe':
            pipe.send({'listener_still_bound':listener.getsockname()[1]==PORT})
        elif command=='stop':
            break
    listener.close()
    os.close(fd)
    pipe.close()

if __name__=='__main__':
    initial={name:Path('/proc/sys/net/vsock/'+name).read_text() for name in ['ns_mode','child_ns_mode']}
    children=[]
    try:
        records=[]
        for _ in range(2):
            parent,remote=multiprocessing.Pipe()
            process=multiprocessing.Process(target=child,args=(remote,))
            process.start()
            remote.close()
            assert parent.poll(20),'namespace child did not become ready'
            records.append(parent.recv())
            children.append((process,parent))
        assert records[0]['namespace']!=records[1]['namespace']
        assert all(row['mode']=='local' and row['same_namespace_duplicate_refused'] for row in records)
        children[0][1].send('stop')
        children[0][0].join(10)
        assert children[0][0].exitcode==0
        children[1][1].send('observe')
        survivor=children[1][1].recv()
        assert survivor['listener_still_bound']
        children[1][1].send('stop')
        children[1][0].join(10)
        assert children[1][0].exitcode==0
        final={name:Path('/proc/sys/net/vsock/'+name).read_text() for name in initial}
        assert initial==final
        print(json.dumps({'kernel':os.uname().release,'concurrent_local_namespaces':records,'survivor_unchanged':survivor,'initial_namespace_modes_unchanged':initial==final,'both_owned_processes_reaped':True},sort_keys=True))
    finally:
        for process,pipe in children:
            if process.is_alive():
                process.terminate()
            process.join(10)
            pipe.close()
