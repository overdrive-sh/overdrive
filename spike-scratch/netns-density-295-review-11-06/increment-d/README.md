# Step 11-06 observed UDP owner release contrast — Increment D

This is the necessity contrast for increment C's reproduced exhaustion after
immediate application close. The production init, GuestKit, default image,
host echo protocol, 1,100 ordinary UDP sockets and three-second audit wait are
the same. The one workload change is to receive each socket's exact echo
before closing it, so the production guest owner necessarily observed that
slot before socket release. No production or permanent test source changes.

Prediction: every one of the 1,100 control exchanges echoes, and the fresh
held socket after three audit periods also echoes. A refusal, missing echo or
incorrect echo falsifies this contrast; no product delay is added.

Run `bash spike-scratch/netns-density-295-review-11-06/increment-d/run.sh default-kit`.
All module, route and BPF effects are confined to a disposable Cloud
Hypervisor guest on the existing leased Metal runner. Full raw output,
console and exit status are retained under `evidence/reruns/`; build output
is ignored.
