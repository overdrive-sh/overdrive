import json,sys
L=sys.argv[1]
try: d=json.load(open(f"/tmp/v31-work/evidence/{L}.json"))
except Exception as e:
    print(f"{L}: NO JSON {e}"); sys.exit(1)
t=d["trials"]; g=d["guest"]; h=d["host"]
def tot(k): return g.get(k,0)+h.get(k,0)
viol={k:tot(k) for k in set(g)|set(h) if k.startswith("VIOLATION") and tot(k)}
aborts={k[6:]:tot(k) for k in set(g)|set(h) if k.startswith("abort_") and tot(k)}
b=d["bpf_counters"]
extra=f" | calls={b.get('fexit_send_calls',0)+b.get('fexit_send_untracked_target',0)} rec_misses A/B={b.get('fexit_recursion_misses')}/{b.get('fexit_b_recursion_misses')} inj={b.get('injected_misses')} dis(a<b,b<a)=({tot('counter_a_short_of_b')},{tot('counter_b_short_of_a')}) fire(fexit/poke)={b.get('fire_fexit')}/{b.get('fire_poke')} owner_cas={tot('complete_owner_cas')} rec_fexit={tot('complete_record_fexit')} rec_poke={tot('complete_record_poke')}"
print(f"{L}: trials={t['total']} pass={t['pass']} reset={t.get('reset',0)} fail={t['fail']} loss_ms={d.get('ctl_loss_at_ms')} wall={d['wall_s']:.0f}s | clean={tot('flows_clean')} aborted={tot('flows_aborted')} aborts={aborts} | ctl_lost={tot('ctl_lost')} paired_at_loss={tot('paired_at_ctl_loss')} paired_clean_after={tot('paired_flows_clean_after_ctl_loss')} intake_after={g.get('intake_after_ctl_loss',0)} | viol={viol} | live_end={d['guest_live_flows_at_end']}/{d['host_live_flows_at_end']} cells_free={d['host_cells']['free_cells']}/{d['host_cells']['cell_owner']}"+extra)
for e in t["examples"][:1]: print("  EX", json.dumps(e)[:300])
