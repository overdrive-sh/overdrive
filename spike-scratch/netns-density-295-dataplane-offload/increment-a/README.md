# S-05-C GuestKit offload falsification

This bounded diagnostic reuses the existing GuestKit public fixture to run only
`adding_listener_on_resubmit_converges_without_breaking_existing` inside a
disposable guest. It stages the exact native module/tool closure plus
`ethtool`, applies the existing private topology's fail-closed four-end
TX-checksum-offload precondition, and retains the unchanged case output. It
does not add a selectable runner group, CI lane, scenario, executor, product
API, or physical-host network effect.

Run only after `cargo xtask guest-kit-prepare` on the qualified Metal host:

```text
cargo run --manifest-path \
  spike-scratch/netns-density-295-dataplane-offload/increment-a/Cargo.toml
```

## Result

Durable Metal `6c6a56320ecafbfd509a3609` exited zero. All four veth ends read
back `tx-checksumming: off`; the exact unchanged UDP/8082, UDP/8081 and
TCP/8080 clients received their expected markers; libtest reported one pass
and zero failures/ignored. This confirms the missing GuestKit `ethtool`
payload as the fixture cause and does not establish a production defect.

Raw receipt:
`evidence/metal-6c6a56320ecafbfd509a3609.log`, SHA-256
`99c07ea3b978d90025f0fc4fb379c6dc3b8ba8570c579a86aa2b4e2791545c82`.
