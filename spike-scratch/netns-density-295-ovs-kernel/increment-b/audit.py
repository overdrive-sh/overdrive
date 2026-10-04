"""Independent read-only Generic Netlink dumper. No packet execute operation."""
import socket,struct,json,sys
def attrs(b):
 d={};p=0
 while p+4<=len(b):
  n,t=struct.unpack_from('HH',b,p)
  if n<4 or p+n>len(b):raise ValueError('invalid nla')
  d[t&0x3fff]=b[p+4:p+n];p+=(n+3)&~3
 return d
def attr(t,b):return struct.pack('HH',len(b)+4,t)+b+b'\0'*((-len(b))%4)
s=socket.socket(socket.AF_NETLINK,socket.SOCK_RAW,16);s.bind((0,0));s.settimeout(5);seq=0
def req(f,c,v,dp=None,a=b'',dump=False):
 global seq
 seq+=1;p=bytes([c,v,0,0])+(struct.pack('I',dp)if dp is not None else b'')+a;s.send(struct.pack('IHHII',len(p)+16,f,0x301 if dump else 5,seq,0)+p);out=[]
 while True:
  b=s.recv(1<<20);off=0
  while off+16<=len(b):
   n,t,flags,q,pid=struct.unpack_from('IHHII',b,off);body=b[off+16:off+n];off+=(n+3)&~3
   if q!=seq:continue
   if t==2:
    error=struct.unpack_from('i',body)[0]
    if error:raise OSError(-error,'kernel netlink rejected read-only request')
    return out
   if t==3:return out
   out.append(body)
def family(n):return struct.unpack('H',attrs(req(16,3,2,a=attr(2,n.encode()+b'\0'))[0][4:])[1])[0]
dp=int(sys.argv[1]);dpf=family('ovs_datapath');vpf=family('ovs_vport');flf=family('ovs_flow')
dps=req(dpf,3,2,0,dump=True)
receipt={'datapaths':[{'ifindex':struct.unpack_from('I',b,4)[0],'attributes_hex':{str(k):v.hex() for k,v in attrs(b[8:]).items()}}for b in dps]}
if dp:
 ports=req(vpf,3,1,dp,dump=True);flows=req(flf,3,1,dp,dump=True)
 receipt['ports']=[]
 for b in ports:
  a=attrs(b[8:]);receipt['ports'].append({'port':struct.unpack('I',a[1])[0],'type':struct.unpack('I',a[2])[0],'name':a[3].rstrip(b'\0').decode(),'upcall_pids':list(struct.unpack('I'*(len(a[5])//4),a[5])),'ifindex':struct.unpack('I',a[8])[0]if 8 in a else None,'stats':list(struct.unpack('Q'*8,a[6])),'attributes_hex':{str(k):v.hex() for k,v in a.items()}})
 receipt['flows']=[]
 for b in flows:
  a=attrs(b[8:]);actions=attrs(a[2]);assert set(actions)=={1} and len(actions[1])==4
  receipt['flows'].append({'output':struct.unpack('I',actions[1])[0],'attributes_hex':{str(k):v.hex()for k,v in a.items()}})
 receipt['stats']=list(struct.unpack('Q'*4,attrs(req(dpf,3,2,dp)[0][8:])[3]))
print(json.dumps(receipt,sort_keys=True))
