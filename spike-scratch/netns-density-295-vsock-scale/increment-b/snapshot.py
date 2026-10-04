import json, pathlib
def snap(prefix):
    result={}
    for key,args in [('links',['ip','-j','-d','link']),('addrs',['ip','-j','address']),('routes',['ip','-j','route','show','table','all']),('rules',['ip','-j','rule']),('nft',['nft','-j','list','ruleset']),('netns',['ip','netns','list']),('bpfmaps',['bpftool','-j','map','show']),('bpflinks',['bpftool','-j','link','show'])]:
        content=raw(prefix+'-'+key+'.json',args)
        try:result[key]=json.loads(content)
        except ValueError:result[key]=content
    for key,path in [('modules','/proc/modules'),('ip-forward','/proc/sys/net/ipv4/ip_forward'),('rp-filter','/proc/sys/net/ipv4/conf/all/rp_filter')]:
        result[key]=pathlib.Path(path).read_text();(OUT/(prefix+'-'+key+'.txt')).write_text(result[key])
    raw(prefix+'-neighbors.json',['ip','-j','-s','neigh','show']);raw(prefix+'-processes.txt',['ps','-e','-o','pid,ppid,comm,args']);raw(prefix+'-bpfpins.txt',['find','/sys/fs/bpf/overdrive','-maxdepth','4','-printf','%P %y %i\n'])
    return result
def normalize(data):
    ignored={'valid_life_time','preferred_life_time','expires','lastuse','used','updated','stats64','gc_timer','stats','cache','packets','bytes'}
    def walk(value):
        if isinstance(value,dict):return {k:walk(v) for k,v in sorted(value.items()) if k not in ignored}
        if isinstance(value,list):return sorted([walk(v) for v in value],key=lambda v:json.dumps(v,sort_keys=True))
        return value
    out=walk(data);out['modules']=sorted(' '.join(s.split()[:2]+s.split()[4:]) for s in data['modules'].splitlines());return out
