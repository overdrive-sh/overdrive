#!/usr/bin/env python3
"""Keep private exact PCAPs, redact configured target only in shared derivatives, losslessly gzip text."""
import gzip,hashlib,json,pathlib,socket,struct
ROOT=pathlib.Path(__file__).resolve().parents[2];SCRATCH=pathlib.Path(__file__).parent
fields={s.split('=',1)[0]:s.split('=',1)[1].strip().strip('\"').strip("'") for s in (ROOT/'.env').read_text().splitlines() if '=' in s and s.startswith('OVERDRIVE_METAL_')}
original_ip=socket.inet_aton(fields['OVERDRIVE_METAL_TARGET'].split('@')[-1]);replacement_ip=socket.inet_aton('198.18.255.254')
def digest(data):return hashlib.sha256(data).hexdigest()
def checksum(data):
    if len(data)%2:data+=b'\0'
    n=sum(struct.unpack('!'+str(len(data)//2)+'H',data))
    while n>>16:n=(n&65535)+(n>>16)
    return (~n)&65535
records=[]
for inc in sorted(SCRATCH.glob('increment-*')):
    evidence=inc/'evidence'
    for raw in sorted(evidence.glob('*.pcap')):
        data=raw.read_bytes();out=bytearray(data)
        if data[:4]!=b'\xd4\xc3\xb2\xa1':raise RuntimeError('expected little-endian classic microsecond pcap')
        if struct.unpack_from('<I',data,20)[0]!=276:raise RuntimeError('expected Linux SLL2')
        offset=24;changed=0
        while offset<len(data):
            stamp_sec,stamp_usec,caplen,wirelen=struct.unpack_from('<IIII',data,offset);frameoff=offset+16;frame=bytearray(data[frameoff:frameoff+caplen]);modified=False
            if len(frame)>=40 and frame[:2]==b'\x08\x00':
                ipoff=20;ihl=(frame[ipoff]&15)*4;total=struct.unpack_from('!H',frame,ipoff+2)[0]
                for pos in [ipoff+12,ipoff+16]:
                    if frame[pos:pos+4]==original_ip:frame[pos:pos+4]=replacement_ip;modified=True
                if modified:
                    frame[ipoff+10:ipoff+12]=b'\0\0';struct.pack_into('!H',frame,ipoff+10,checksum(bytes(frame[ipoff:ipoff+ihl])))
                    # This capture contains complete unfragmented host TCP/UDP frames.
                    fragment=struct.unpack_from('!H',frame,ipoff+6)[0]&0x3fff
                    if total+ipoff<=len(frame) and fragment==0 and frame[ipoff+9] in [6,17]:
                        protocol=frame[ipoff+9];transport=ipoff+ihl;checkoff=transport+(16 if protocol==6 else 6)
                        frame[checkoff:checkoff+2]=b'\0\0';payload=bytes(frame[transport:ipoff+total]);pseudo=bytes(frame[ipoff+12:ipoff+20])+struct.pack('!BBH',0,protocol,len(payload));value=checksum(pseudo+payload)
                        struct.pack_into('!H',frame,checkoff,value or (65535 if protocol==17 else 0))
            elif len(frame)>=48 and frame[:2]==b'\x08\x06':
                for pos in [34,44]:
                    if frame[pos:pos+4]==original_ip:frame[pos:pos+4]=replacement_ip;modified=True
            if modified:out[frameoff:frameoff+caplen]=frame;changed+=1
            offset=frameoff+caplen
        private=SCRATCH/'out/raw-captures'/inc.name/raw.name;private.parent.mkdir(parents=True,exist_ok=True)
        if private.exists():raise RuntimeError('refuse private raw overwrite')
        private.write_bytes(data)
        dest=raw.with_name('all-interfaces.target-redacted.pcap.gz');encoded=gzip.compress(bytes(out),mtime=0);dest.write_bytes(encoded)
        assert gzip.decompress(encoded)==bytes(out)
        records.append({'attempt':inc.name,'original_path':str(private.relative_to(SCRATCH)),'original_sha256':digest(data),'shared_path':str(dest.relative_to(SCRATCH)),'shared_uncompressed_sha256':digest(out),'shared_compressed_sha256':digest(encoded),'bytes_before':len(data),'bytes_after':len(out),'redacted_packets':changed,'transformation':'Only configured native target IPv4 -> 198.18.255.254; IPv4 and complete TCP/UDP checksums recomputed; frame sizes/timestamps otherwise retained. Shared derivative is NOT raw wire evidence. Exact original is also in private per-attempt archive under ignored out.'})
        raw.unlink()
    for raw in sorted(evidence.iterdir()):
        if not raw.is_file() or raw.suffix=='.gz' or raw.stat().st_size<16384:continue
        data=raw.read_bytes();encoded=gzip.compress(data,mtime=0);dest=raw.with_name(raw.name+'.gz')
        if dest.exists():raise RuntimeError('refuse compression overwrite')
        dest.write_bytes(encoded);assert gzip.decompress(encoded)==data
        records.append({'attempt':inc.name,'original_path':str(raw.relative_to(SCRATCH)),'shared_path':str(dest.relative_to(SCRATCH)),'original_sha256':digest(data),'compressed_sha256':digest(encoded),'original_bytes':len(data),'compressed_bytes':len(encoded),'roundtrip_equal':True})
        raw.unlink()
(SCRATCH/'evidence-preservation.json').write_text(json.dumps(records,indent=2,sort_keys=True))
print(json.dumps({'artifacts':len(records),'pcap_derivatives':sum('redacted_packets' in r for r in records),'lossless_text_roundtrips_verified':sum(r.get('roundtrip_equal',False) for r in records)}))
