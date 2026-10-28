# Dummy automatic-device cleanup proof

This bounded GuestKit proof traces the stock dummy module from an absent
pre-load state. It loads the same module closure as the existing
`cgroup-default` worker guest, reconciles the empty parameter load with Linux
7.0's source default `numdummies=1` (whose parameter permission `0` means no
sysfs parameter file), records the automatic unconfigured `dummy0`, removes
only that freshly proven fixture-owned device, and leaves the module loaded.

The proof then creates and cleans the existing STEP 13-04 named dummy sink,
runs the complete STEP 13-05 qualified-node audit, executes the unchanged
`vsock_legb_dial::a_same_node_mesh_dial_is_marked_source_bound_and_reaches_leg_c`
body, and repeats the audit.

It adds no executable dependency, production behavior, test body, scenario,
selector, group, lane, executor, API, or physical-host network state.

## Result

Durable Metal `bec78c8640bd312348e34623` exited zero. The first guest command
observed only `dummy0` and `lo`, with the dummy module already loaded by the
scripted PID 1. The automatic device was dummy-kind, down, unmastered, and had
no IPv4/IPv6 address or route. Cleanup left only `lo` while the module stayed
loaded.

The proof then created the STEP 13-04 `ovd-gk-route` sink, observed its exact
default route and source, removed it, and returned to only `lo`. Both STEP
13-05 audits passed around the unchanged exact leg-B body. That body reported
one pass, zero failed/ignored, and retained loopback source plus fwmark 1.

The receipt ended `DUMMY_AUTO_DEVICE_CLEANUP_LEGB_PASS`.

Raw receipt: `evidence/metal-bec78c8640bd312348e34623.log`, SHA-256
`8ebdb62d5b18396bda26dd67f66b5715b62c5ef9e3625009c8ba122dbd16da7c`.
