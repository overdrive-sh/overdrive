# UDP capture account diagnosis

This bounded diagnostic reuses the existing GuestKit public fixture and exact
dataplane-native payload. Before running
`reverse_nat_udp_e2e::every_udp_reply_independently_source_rewritten`, it
creates one guest-private netns/veth pair and executes the fixture's exact
`ip netns exec … tcpdump -U -n -i … -w … udp` shape. It retains the guest
account files, argv, exit status, stdout, and stderr so a capture-account or
binary/runtime failure is proven before any support correction.

It adds no selectable group, CI lane, scenario, executor, product API, or
physical-host network effect.

Run only after `cargo xtask guest-kit-prepare` on the qualified Metal host:

```text
cargo run --manifest-path \
  spike-scratch/netns-density-295-dataplane-offload/increment-b/Cargo.toml
```

## Results

Without guest account files, durable Metal `9d95728658b949ca36438ff0`
reproduced the exact inner execution and retained:

```text
/etc/passwd ABSENT
/etc/group ABSENT
state=exited-before-readiness-window returncode=1
stderr="tcpdump: Couldn't find user 'tcpdump'\n"
```

With only the qualified host's existing unprivileged tcpdump identity staged
inside the disposable guest, durable Metal `ed145fdfdd75dca1dbfe8c7b` exited
zero. The same diagnostic remained live, emitted `listening on`, and dropped
to `tcpdump:x:985:985`. The unchanged exact native body then captured three
outbound query witnesses, received all three replies, observed every reply
source as `10.0.0.1`, and reported one pass with zero failed or ignored.

Raw receipts:

- `evidence/metal-9d95728658b949ca36438ff0-no-account.log`, SHA-256
  `10cde4d5090066d37d99f011b9a7cf58d36d10c13b21e1ea5335660dd3f972b6`;
- `evidence/metal-ed145fdfdd75dca1dbfe8c7b-with-account.log`, SHA-256
  `9bb8dc6b24eba7f3d61c2315e810e4c3e1fe8765515e514c290bab390402aabf`.
