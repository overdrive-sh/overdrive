import json,sys
L,RC,ARGS=sys.argv[1],sys.argv[2],sys.argv[3]
print(f"=== {L} rc={RC} args: {ARGS}")
try: d=json.load(open(f"/tmp/v31-work/evidence/{L}.json"))
except Exception as e:
    print("NO JSON", e); print(open(f"/tmp/v31-work/evidence/{L}.err").read()[:800]); sys.exit(1)
t=d["trials"]; print(f"trials total={t['total']} pass={t['pass']} reset={t.get('reset',0)} fail={t['fail']} wall_s={d['wall_s']:.1f} ctl_loss_at_ms={d.get('ctl_loss_at_ms')} uname={d['uname']}")
def acct(r):
    c=d[r]; own=c.get('complete_owner_cas',0); rf=c.get('complete_record_fexit',0); rp=c.get('complete_record_poke',0)
    comp=own+rf+rp; clean=c.get('flows_clean',0); ab=c.get('flows_aborted',0)
    viol={k:v for k,v in c.items() if k.startswith('VIOLATION')}
    aborts={k[6:]:v for k,v in c.items() if k.startswith('abort_')}
    tcp=c.get('tcp_eof_events',0); vs=c.get('vsock_eof_events',0)
    print(f" {r}: started={c.get('flows_started',0)} clean={clean} aborted={ab} completions={comp} (owner_cas={own} rec_fexit={rf} rec_poke={rp}) expected_if_all_clean={2*clean} records={c.get('records',0)} owner_lost_cas={c.get('owner_lost_cas',0)}")
    print(f"   shutdown records: 0x2={c.get('shut_0x2',0)} 0x3={c.get('shut_0x3',0)} rst={c.get('shut_rst',0)} other={c.get('shut_other',0)} rdhup_v_without_record={c.get('rdhup_v_without_record',0)}")
    print(f"   gap@EOF: tcp_src sent<goal {c.get('tcp_eof_sent_lt_goal',0)}/{tcp} (deficit_max={c.get('tcp_eof_deficit_max',0)}B) fwd<goal {c.get('tcp_eof_fwd_lt_goal',0)}/{tcp}; vsock_src sent<goal {c.get('vsock_eof_sent_lt_goal',0)}/{vs}")
    print(f"   backstop: looks={c.get('backstop_looks',0)} rises={c.get('backstop_miss_rises',0)} pokes={c.get('backstop_pokes',0)} arm_pokes={c.get('arm_pokes',0)} owner_waits={c.get('owner_waits_for_record',0)} counters_disagree(a<b,b<a)=({c.get('counter_a_short_of_b',0)},{c.get('counter_b_short_of_a',0)})")
    print(f"   ctl: lost={c.get('ctl_lost',0)} paired_at_loss={c.get('paired_at_ctl_loss',0)} paired_clean_after_loss={c.get('paired_flows_clean_after_ctl_loss',0)} intake_after_loss={c.get('intake_after_ctl_loss',0)}")
    print(f"   exactness: tcp_fwd_ne_goal={c.get('tcp_fwd_ne_goal_at_completion',0)} record_sent_ne_goal={c.get('record_sent_ne_goal',0)} VIOLATIONS={viol} aborts={aborts}")
for r in ("guest","host"): acct(r)
b=d['bpf_counters']
print(f" live_at_end guest={d['guest_live_flows_at_end']} host={d['host_live_flows_at_end']} cells={d['guest_cells']} {d['host_cells']}")
print(f" bpf: ringbuf_fail={b.get('ringbuf_output_fail')} install_fail={b.get('install_fail')} redirect_fail={b.get('verdict_redirect_fail')} noroute={b.get('verdict_noroute')} level_unreadable={b.get('level_unreadable')} send_err_hist={b.get('send_err_hist')}")
print(f"      fexit run_cnt(A)={b.get('fexit_run_cnt')} recursion_misses A={b.get('fexit_recursion_misses')} B={b.get('fexit_b_recursion_misses')} injected={b.get('injected_misses')} poke_runs={b.get('poke_runs')} fire_poke={b.get('fire_poke')} fire_fexit={b.get('fire_fexit')} shut_ring_drops={b.get('tracepoint_drops')}/{b.get('shut_ring_drop_host')}")
for e in t["examples"][:2]: print(" EX", json.dumps(e)[:400])
def nst(f):
    d={}
    try:
        for l in open(f):
            p=l.split()
            if len(p)>=2:
                try: d[p[0]]=int(p[1])
                except: pass
    except FileNotFoundError: pass
    return d
n0,n1=nst(f"/tmp/v31-work/evidence/{L}.nstat0"),nst(f"/tmp/v31-work/evidence/{L}.nstat1")
keys=["TcpRetransSegs","TcpExtTCPWinProbe","TcpExtTCPTimeouts","TcpExtTCPRcvQDrop","TcpExtTCPBacklogDrop","TcpExtTCPAbortOnData","TcpExtTCPAbortOnClose"]
print(" nstat delta:", {k:n1.get(k,0)-n0.get(k,0) for k in keys if n1.get(k,0)-n0.get(k,0)})
