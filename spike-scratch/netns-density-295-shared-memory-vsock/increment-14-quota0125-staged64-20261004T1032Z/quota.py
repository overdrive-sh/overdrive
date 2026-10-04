"""Probe-owned cgroup-v2 CPU budget. Never modifies a foreign subtree/controller."""
import json,os,pathlib,time
ROOT=pathlib.Path('/sys/fs/cgroup')
class Quota:
    def __init__(self,out,tag,quota=12500,period=100000,aggregate=800000):
        self.out=out;self.parent=ROOT/tag;self.children={};self.quota=quota;self.period=period;self.aggregate=aggregate
    def prepare(self):
        before={name:(ROOT/name).read_text() for name in ['cgroup.controllers','cgroup.subtree_control','cpuset.cpus.effective'] if (ROOT/name).exists()};(self.out/'quota-root-before.json').write_text(json.dumps(before,indent=2))
        if 'cpu' not in before.get('cgroup.subtree_control','').split():raise RuntimeError('GENUINE_BLOCKER: cpu controller not already enabled in root subtree; foreign root is not modified')
        self.parent.mkdir(exist_ok=False)
        (self.parent/'cpu.max').write_text(str(self.aggregate)+' '+str(self.period))
        (self.parent/'cgroup.subtree_control').write_text('+cpu')
        (self.out/'quota-owned-parent.json').write_text(json.dumps({'path':str(self.parent),'cpu.max':(self.parent/'cpu.max').read_text(),'cgroup.subtree_control':(self.parent/'cgroup.subtree_control').read_text(),'cpuset.cpus.effective':(self.parent/'cpuset.cpus.effective').read_text() if (self.parent/'cpuset.cpus.effective').exists() else 'cpuset controller not enabled here; inherit scheduler affinity, sampled per VMM'},indent=2))
    def child(self,role):
        group=self.parent/role;group.mkdir(exist_ok=False);(group/'cpu.max').write_text(str(self.quota)+' '+str(self.period));self.children[role]=group
        return group
    @staticmethod
    def before_exec(group):
        # This runs only in the freshly forked owned child, before CH exec/guest CPU.
        (group/'cgroup.procs').write_text(str(os.getpid()))
    def readback(self,role,pid):
        group=self.children[role];expected='/'+str(group.relative_to(ROOT));tasks=[]
        for task in pathlib.Path('/proc/'+str(pid)+'/task').iterdir():
            value=(task/'cgroup').read_text();tasks.append({'tid':int(task.name),'cgroup':value,'matched':value.strip()=='0::'+expected})
        if not tasks or not all(t['matched'] for t in tasks):raise RuntimeError('VMM thread escaped quota cgroup '+role)
        ancestors=[];current=group
        while True:
            value={'path':str(current),'cpu.max':(current/'cpu.max').read_text() if (current/'cpu.max').exists() else 'root: no cpu.max limit','cpu.stat':(current/'cpu.stat').read_text(),'cpuset.cpus.effective':(current/'cpuset.cpus.effective').read_text() if (current/'cpuset.cpus.effective').exists() else None};ancestors.append(value)
            if current==ROOT:break
            current=current.parent
        return {'scheduler_affinity':next(s for s in pathlib.Path('/proc/'+str(pid)+'/status').read_text().splitlines() if s.startswith('Cpus_allowed_list:')),'online_host_cpus':pathlib.Path('/sys/devices/system/cpu/online').read_text(),'role':role,'pid':pid,'path':str(group),'cpu.max':(group/'cpu.max').read_text(),'cpu.stat':(group/'cpu.stat').read_text(),'cgroup.procs':(group/'cgroup.procs').read_text(),'cgroup.threads':(group/'cgroup.threads').read_text(),'all_vmm_threads_in_owned_cgroup':True,'tasks':tasks,'ancestors':ancestors,'host_cpu_quota_logical_cpus':self.quota/self.period,'guest_virtual_cpu_topology':1}
    def cleanup(self):
        result=[]
        for role,group in self.children.items():
            if not group.exists():continue
            state={'role':role,'path':str(group),'cpu.max':(group/'cpu.max').read_text(),'cpu.stat':(group/'cpu.stat').read_text(),'cgroup.events':(group/'cgroup.events').read_text(),'cgroup.procs':(group/'cgroup.procs').read_text()}
            result.append(state)
            if state['cgroup.procs'].strip():raise RuntimeError('owned cgroup still populated '+role)
            group.rmdir()
        (self.out/'quota-final-stats-and-cleanup.json').write_text(json.dumps(result,indent=2))
        if self.parent.exists():self.parent.rmdir()
        (self.out/'quota-cleanup-parent.json').write_text(json.dumps({'owned_parent':str(self.parent),'exists':self.parent.exists(),'exact_owned_children_removed':len(result)},indent=2))
