# Increment A — native vDPA / SR-IOV inventory

This is the retained, self-contained source and raw evidence for the bounded
`netns-density-295` native feasibility spike.

Question: can the actual metal NIC/platform support hardware-backed vDPA or
direct SR-IOV guest networking with a viable endpoint-count path for the
required density?

The script is read-only. It never loads a module, changes a devlink parameter,
enables a VF, binds or unbinds a PCI function, installs/configures OVS, resets a
NIC, changes firmware, or changes networking.

Executed through the canonical runner:

```sh
cargo xtask metal run -- \
  bash spike-scratch/netns-density-295-vdpa-sriov/increment-a/inventory.sh
```

The raw stdout is retained losslessly in
`evidence/inventory.raw.log.gz`. Its compressed SHA-256 is
`83745d24f24c6c46bb83b1cec0a9ca0a9e60324fb586b9eafe6d20ac5466161f`;
the decompressed 236,098 bytes have SHA-256
`70e031b9a6372a868865bf859f5c2d8253671b25f6daf69821aa3d118aca95bd`.
The command's canonical lease/preflight wrapper and selected findings are
recorded in
`docs/feature/netns-density-295/spike/findings-vdpa-sriov-metal.md`.

Verdict: `DOESN'T WORK` on the current metal configuration. The software
stack is present, but the host exposes no SR-IOV VFs or hardware vDPA device.
The active management interface shares its dual-port BCM57414 with the only
function that superficially looks spare; enabling SR-IOV is a permanent shared
NIC-NVM change and was not attempted.
