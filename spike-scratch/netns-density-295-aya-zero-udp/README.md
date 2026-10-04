# Aya Rust zero-length UDP proof

**WORKS both directions on stock 7.0.0-29:** SK_SKB private prefix encoding plus
TCX packet-boundary decoding. Two real stock vhost devices/CIDs/shared memory
contexts each pass 0,0,1,0,1431,59000,0 both directions. Eight actual empty
vhost messages and eight ordinary UDP empty receives reconcile; real packet
headers show IP length 28 and UDP length 8. Repeated/collision/foreign/invalid
frame/mapping-removal cases and explicit cleanup are retained.

All BPF runs only in a private KVM host-role VM on the qualified metal fixture;
no custom module, TAP, kernel patch, userspace proxy payload relay or production
change. This is a focused native mechanism proof, not ordinary guest networking,
UDP scale, full UDP length-range, checksum/offload or zero-copy validation.

See [findings](../../docs/feature/netns-density-295/spike/aya-zero-udp-findings.md),
[actual receipts](increment-f/evidence/native-serial.log),
[reconciliation](increment-f/evidence/receipt-summary.derived.json) and
[cleanup readback](post-release-readback.json).

Preserved increments a-f record compile, native candidate, lease and verifier
failures as well as the completed proof. Re-run by copying a source increment
to a new ID and invoking `python3 launch.py increment-new-id`. Existing output
IDs refuse overwrite. Sources/evidence are retained; builds/images stay ignored.
