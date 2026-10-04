#!/usr/bin/env python3
"""Read stock config/module symbols only. No BPF program is built or loaded."""
import hashlib,json,pathlib,subprocess,time
base=pathlib.Path(__file__).resolve().parent
commands=[['uname','-r'],['cat','/boot/config-7.0.0-34-generic'],['nm','-a',str(base/'out/increment-h/rootfs/vsock.ko')],['nm','-a',str(base/'out/increment-h/rootfs/vmw_vsock_virtio_transport_common.ko')]]
receipts=[]
for argv in commands:
 p=subprocess.run(argv,text=True,stdout=subprocess.PIPE,stderr=subprocess.STDOUT)
 lines=p.stdout.splitlines()
 if argv[0]=='cat':lines=[l for l in lines if any(s in l for s in ['CONFIG_BPF','CONFIG_NET_SOCK_MSG','CONFIG_VSOCKETS','CONFIG_VHOST_VSOCK'])]
 if argv[0]=='nm':lines=[l for l in lines if any(s in l for s in ['vsock_bpf','psock','read_skb'])]
 receipts.append({'argv':argv,'exit':p.returncode,'selected_readonly_output':lines,'unfiltered_output_sha256':hashlib.sha256(p.stdout.encode()).hexdigest()})
(base/'stock-capability-readback.json').write_text(json.dumps({'timestamp_epoch_s':time.time(),'activity':'read-only config and object symbols; NOT BPF forwarding validation','receipts':receipts},indent=2))
print(json.dumps(receipts,indent=2))
