import json,sys
L,RC,ARGS=sys.argv[1],sys.argv[2],sys.argv[3]
print(f"=== {L} rc={RC} args: {ARGS}")
try: d=json.load(open(f"/tmp/v27-work/evidence/{L}.json"))
except Exception as e:
    print("NO JSON", e); print(open(f"/tmp/v27-work/evidence/{L}.err").read()[:600]); sys.exit(1)
t=d["trials"]; print(f"trials total={t['total']} pass={t['pass']} fail={t['fail']} wall_s={d['wall_s']:.1f}")
def acct(r):
    c=d[r]; own=c.get('complete_owner_cas',0); rf=c.get('complete_record_fexit',0); rs=c.get('complete_record_state_cb',0)
    comp=own+rf+rs; clean=c.get('flows_clean',0); ab=c.get('flows_aborted',0)
    viol={k:v for k,v in c.items() if k.startswith('VIOLATION')}
    aborts={k:v for k,v in c.items() if k.startswith('abort_')}
    tcp=c.get('tcp_eof_events',0); vs=c.get('vsock_eof_events',0)
    print(f" {r}: started={c.get('flows_started',0)} clean={clean} aborted={ab} completions={comp} (owner_cas={own} rec_fexit={rf} rec_state_cb={rs}) expected_if_all_clean={2*clean} records={c.get('records',0)} owner_lost_cas={c.get('owner_lost_cas',0)}")
    print(f"   gap@EPOLLRDHUP: tcp_src sent<goal {c.get('tcp_eof_sent_lt_goal',0)}/{tcp} (deficit_max={c.get('tcp_eof_deficit_max',0)}B) fwd<goal {c.get('tcp_eof_fwd_lt_goal',0)}/{tcp}; vsock_src sent<goal {c.get('vsock_eof_sent_lt_goal',0)}/{vs}")
    print(f"   exactness: tcp_fwd_ne_goal={c.get('tcp_fwd_ne_goal_at_completion',0)} vsock_fwd_ne_peer_count={c.get('vsock_fwd_ne_peer_count_at_completion',0)} record_sent_ne_goal={c.get('record_sent_ne_goal',0)} record_before_accept={c.get('record_before_accept',0)} VIOLATIONS={viol} aborts={aborts}")
for r in ("guest","host"): acct(r)
print(f" live_at_end guest={d['guest_live_flows_at_end']} host={d['host_live_flows_at_end']}  bpf: ringbuf_fail={d['bpf_counters'].get('ringbuf_output_fail')} install_fail={d['bpf_counters'].get('install_fail')} redirect_fail={d['bpf_counters'].get('verdict_redirect_fail')} noroute={d['bpf_counters'].get('verdict_noroute')} send_err_hist={d['bpf_counters'].get('send_err_hist')} fexit_recursion_misses={d['bpf_counters'].get('fexit_recursion_misses')} fexit_run_cnt={d['bpf_counters'].get('fexit_run_cnt')}")
for e in t["examples"][:2]: print(" EX", json.dumps(e)[:300])

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
n0,n1=nst(f"/tmp/v27-work/evidence/{L}.nstat0"),nst(f"/tmp/v27-work/evidence/{L}.nstat1")
keys=["TcpRetransSegs","TcpExtTCPDSACKOldSent","TcpExtTCPDSACKOfoSent","TcpExtTCPDSACKRecv","TcpExtTCPZeroWindowDrop","TcpExtTCPRcvQDrop","TcpExtPruneCalled","TcpExtTCPBacklogDrop","TcpExtTCPWinProbe","TcpExtTCPTimeouts","TcpExtTCPSpuriousRTOs","TcpExtTCPFastRetrans","TcpExtTCPSlowStartRetrans","TcpExtRcvPruned","TcpExtOfoPruned","TcpExtTCPRcvCollapsed","TcpExtTCPRcvCoalesce"]
print(" nstat delta:", {k:n1.get(k,0)-n0.get(k,0) for k in keys if n1.get(k,0)-n0.get(k,0)})
