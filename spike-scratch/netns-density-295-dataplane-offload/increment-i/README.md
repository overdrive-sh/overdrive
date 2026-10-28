# LEGB route-readback normalization proof

This bounded GuestKit proof runs the existing
`vsock_legb_dial::a_same_node_mesh_dial_is_marked_source_bound_and_reaches_leg_c`
body between the exact STEP13-05 pre/post audits. The route readback removes
only one trailing iproute separator before the existing strict route-shape
match. Every route attribute, route-localnet, non-gateway, default/uplink,
fwmark, and table-100 assertion remains unchanged.

It adds no executable dependency, scenario, selector, lane, production API,
or physical-host network state.

## Result

Durable Metal `6eeb8cde64128fd1c0daa173` exited zero. Before and after the
unchanged exact body, the guest rendered the route with one trailing ASCII
space:

```text
gateway_raw=<local 100.95.0.1 dev lo proto 247 scope host >
gateway_normalized=<local 100.95.0.1 dev lo proto 247 scope host>
```

Both audits also retained `route_localnet=0`, an empty main table, only `lo`,
no fwmark-1/table-100 residue, and no route to the non-gateway prefix address.
The exact existing leg-B body passed with the client socket sourced from
`127.0.0.1` and marked `1`, and the receipt ended
`LEGB_AUDIT_BOUNDED_REPLAY_PASS`.

Raw receipt: `evidence/metal-6eeb8cde64128fd1c0daa173.log`, SHA-256
`cb0c1b578b3d869d3965d474bbd8fc36070356808a8d7a5e7bbceee090dce63e`.
