"""Independent offline check of native packet bytes and control wire capture."""
import pathlib,json,gzip,struct,collections,hashlib
ROOT=pathlib.Path(__file__).resolve().parent;E=ROOT/'increment-d/evidence'
def loadbytes(name):
 p=E/name
 return p.read_bytes()if p.exists()else gzip.decompress(p.with_name(p.name+'.gz').read_bytes())
def checksum(b):
 if len(b)%2:b+=b'\0'
 n=sum(struct.unpack('!'+str(len(b)//2)+'H',b))
 while n>>16:n=(n&65535)+(n>>16)
 return (~n)&65535
def ip(i):return struct.pack('!I',0xc6120002+i)
def mac(i):return b'\x02\x00'+ip(i)
gw=bytes.fromhex('02aa29500001');gateway=bytes.fromhex('c6120001');packet_results=[]
for n in [4,1024,4096,8192,16384]:
 seen=collections.Counter()
 for line in loadbytes(f'gateway-traffic-{n}.jsonl').splitlines():
  d=json.loads(line);i=d['index'];assert 0<=i<n;a=bytes.fromhex(d['request_hex']);b=bytes.fromhex(d['reply_hex']);seen[(i,d['protocol'])]+=1
  assert a[6:12]==mac(i)and b[:6]==mac(i)and b[6:12]==gw
  if d['protocol']=='arp':
   assert a[:6]==b'\xff'*6 and a[12:22]==bytes.fromhex('08060001080006040001')and b[12:22]==bytes.fromhex('08060001080006040002')
   assert a[22:28]==mac(i)and a[28:32]==ip(i)and a[38:42]==gateway
   assert b[22:28]==gw and b[28:32]==gateway and b[32:38]==mac(i)and b[38:42]==ip(i)
  else:
   assert a[:6]==gw and a[12:14]==b[12:14]==b'\x08\x00'and a[23]==b[23]==1
   assert a[26:30]==ip(i)and a[30:34]==gateway and b[26:30]==gateway and b[30:34]==ip(i)
   assert a[34]==8 and b[34]==0 and a[38:]==b[38:] and checksum(a[14:34])==checksum(b[14:34])==checksum(a[34:])==checksum(b[34:])==0
 assert all(seen[i,'arp']==1 and seen[i,'icmp']==2 for i in range(n))and sum(seen.values())==3*n
 packet_results.append({'population':n,'arp_roundtrips':n,'icmp_payload_roundtrips':2*n,'all_ids_covered':True})
seen=set()
for line in loadbytes('paired-tcp-traffic-16384.jsonl').splitlines():
 d=json.loads(line);i=d['source_index'];peer=d['destination_index'];a=bytes.fromhex(d['sent_hex']);b=bytes.fromhex(d['received_hex'])
 assert i not in seen and peer==i^1 and a==b and len(a)==86 and a[:6]==mac(peer)and a[6:12]==mac(i)
 assert a[12:14]==b'\x08\x00'and a[23]==6 and a[26:30]==ip(i)and a[30:34]==ip(peer)and checksum(a[14:34])==0
 pseudo=a[26:34]+b'\x00\x06'+struct.pack('!H',len(a)-34)+a[34:];assert checksum(pseudo)==0
 assert struct.unpack('!H',a[34:36])[0]==40000+i and struct.unpack('!H',a[36:38])[0]==48123 and struct.unpack('!Q',a[54:62])[0]==i and struct.unpack('!Q',a[62:70])[0]==peer
 seen.add(i)
assert seen==set(range(16384))
def attrs(b):
 out={};p=0
 while p<len(b):
  n,t=struct.unpack_from('HH',b,p);assert n>=4 and p+n<=len(b);t&=0x3fff;assert t not in out;out[t]=b[p+4:p+n];p+=(n+3)&~3
 return out
sent=0;received=0;counts=collections.Counter();userspace_actions=0;packet_execute=0;packet_received=0
for line in loadbytes('netlink-wire.jsonl').splitlines():
 d=json.loads(line);b=bytes.fromhex(d['datagram_hex']);assert len(b)==d['bytes']
 if d['direction']=='sent':
  sent+=1;length,family,flags,sequence,pid=struct.unpack_from('IHHII',b);assert sequence==sent and length==len(b);c=b[16];counts[(family,c)]+=1;packet_execute+=int(family==46 and c==3)
  if family==45 and c==1:
   actions=attrs(attrs(b[24:])[2]);assert set(actions)=={1};userspace_actions+=int(2 in actions)
  if family==44 and c==1:assert attrs(b[24:])[5]==b'\0'*4
 else:
  received+=1;p=0
  while p<len(b):
   length,family=struct.unpack_from('IH',b,p);assert length>=16 and p+length<=len(b);packet_received+=int(family==46);p+=(length+3)&~3
assert sent==81943 and received==163886 and packet_execute==packet_received==userspace_actions==0
post=json.loads((ROOT/'post-release-readback.json').read_text());assert not any(post['known_owned_pids_exist'].values())and not any(post['known_owned_paths_exist'].values())and not post['ovs_userspace_processes']
for c in json.loads((ROOT/'compression-manifest.json').read_text()):
 b=gzip.decompress((ROOT/c['retained_path']).read_bytes());assert len(b)==c['original_bytes']and hashlib.sha256(b).hexdigest()==c['original_sha256']
result={'passed':True,'gateway':packet_results,'transparent_tcp_frames':len(seen),'bidirectional_pairs':len(seen)//2,'netlink_sent_datagrams':sent,'netlink_received_datagrams':received,'packet_execute_requests':packet_execute,'received_packet_family_messages':packet_received,'userspace_actions_in_all_flow_installs':userspace_actions,'request_family_command_counts':{str(k):v for k,v in counts.items()},'post_release_cleanup_passed':True,'lossless_receipt_hashes_passed':True}
(ROOT/'receipt-verification.json').write_text(json.dumps(result,indent=2,sort_keys=True));print(json.dumps(result,sort_keys=True))
