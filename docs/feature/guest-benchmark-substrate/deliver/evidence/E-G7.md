# E-G7 receipt: a guest stalled past the READY/beacon bound, through the family
- 01:10:16Z `metal run --benchmark guest-flow-forwarding` run 58e217e3ee6e494b10b120eb, exit 0, finished 01:12:41Z. Result dir benchmarks/results/guest-flow-forwarding/58e217e3ee6e494b10b120eb (pulled).
- Signal (direct ssh): first VMM seen 01:10:38Z; at 01:10:59Z `sudo -n kill -STOP 3352535` (the VMM then running, 0 s old, state Sl -> Tl). Only that pid.
- Record: replicate block 10 `outcome:failed`, failure `guest run: staging the guest image: guest program: the guest sent no beacon within 60s; console: ...` (the family's mapping of the beacon bound); its console.log retained at artifacts/default/10-0-forwarding/console.log (22 KB); 29 ok, 30 console dirs; terminal `run_finished replicates_ok 29 replicates_failed 1`; report `status: finished`.
- After: probe run 72ac3793a7706319365813bb: no cloud-hypervisor process, no stopped process, 0 vhost-vsock holders.
- "Never reaches READY" through the family: needs a code change (a guest that boots but withholds READY); not produced. GF1 covers it at the function level. Note the failure text names the guest beacon bound ("no beacon within 60s"), not READY.
