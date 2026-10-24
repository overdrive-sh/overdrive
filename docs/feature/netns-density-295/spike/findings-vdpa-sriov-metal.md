# Native metal vDPA / SR-IOV feasibility spike

**Date:** 2026-10-24

**Timebox:** 1 hour maximum; completed within the timebox, with the final retained inventory taking 31.7 seconds including lease/sync/preflight

**Scope:** read-only physical-host inventory and the smallest experiment the inventory permitted

**Substrate:** canonical `cargo xtask metal run --` runner and exclusive host lease

## Verdict: DOESN'T WORK on the current metal configuration

The current metal box cannot run a **hardware-backed vDPA** or **direct SR-IOV VF** guest-network experiment without first changing persistent NIC configuration on the same dual-port card that carries the management connection.

The software stack is ready: Linux 7.0 has PCI IOV, AMD IOMMU, VFIO, vDPA, vendor vDPA modules and `vhost_vdpa`; `iproute2` supplies `vdpa`; Cloud Hypervisor v53 exposes both `--vdpa` and direct `--device` assignment. The hardware endpoint is absent in the current boot:

- the two 25 Gb/s functions are Broadcom BCM57414 ports driven by `bnxt_en`;
- neither exposes `sriov_totalvfs` or a PCI SR-IOV capability;
- both report devlink `enable_sriov`, permanent value `false`;
- `/sys/bus/vdpa` and `/dev/vhost-vdpa-*` are absent, and `vdpa` cannot resolve the vDPA Generic Netlink family;
- Linux 7.0 has no hardware-vDPA driver for the BCM57414/`bnxt_en` device;
- the third NIC, a BCM5720, exposes neither SR-IOV nor vDPA.

No software simulator was substituted. `vdpa_sim_net` would prove only the kernel/VMM plumbing already covered upstream; it cannot prove hardware availability, endpoint cardinality, physical line rate, or the target-density path.

## Question tested

One assumption only:

> Can the actual metal NIC/platform support hardware-backed vDPA or direct SR-IOV guest networking with a viable endpoint-count path for the required density?

**Answer:** not in its current hardware/firmware configuration. A future adapter/configuration may, but this box exposes zero usable hardware vDPA devices and zero SR-IOV VFs today.

## Original density constraint, precisely stated

Vsock was chosen to remove per-VM host networking objects after two earlier density ceilings/cost shapes:

1. The pre-#295 path assigned each VM a project `NetSlot`, per-workload network namespace, veth pair, transit and guest `/30`s, and TAP. `NetSlot` imposed a project-defined 4,096 admission ceiling ([ADR-0117](../../../../docs/product/architecture/adr-0117-initial-shared-bridge-density-target.md)).
2. The first #295 replacement kept one host TAP per VM on one node-local Linux bridge. Native E18 failed with `EXFULL`. Linux 7.0 defines `BR_PORT_BITS = 10`, `BR_MAX_PORTS = 1 << 10`, reserves port 0, and refuses another port when the bitmap is full. The exact limit is therefore 1,023 usable ports on **one Linux bridge**, not 1,023 Linux network interfaces globally. [Pinned constants](https://github.com/torvalds/linux/blob/v7.0/net/bridge/br_private.h#L26-L27), [allocator](https://github.com/torvalds/linux/blob/v7.0/net/bridge/br_if.c#L378-L396), [native E18 record](../deliver/native-e18-bridge-capacity-falsification.md).

Two project probes disprove a universal 1,023-netdevice interpretation:

- [multi-bridge](multi-bridge-findings.md) retained 4,097 TAP interfaces across leaf bridges, then found a distinct untuned neighbor-pressure failure;
- [stock kernel OVS](ovs-kernel-findings.md) retained 16,384 TAP netdevices, queue FDs and NETDEV vports and forwarded bounded synthetic traffic, while retaining all of that O(N) host state and measuring material cleanup cost.

The accepted [vsock attachment](../../../../docs/product/architecture/adr-0145-vsock-kernel-forwarded-guest-transport-replaces-shared-bridge.md) changes resource class: no host TAP, veth, bridge port or per-allocation netdevice; one vhost-vsock device/CID plus sockets/tasks per VM. Final production density/cost measurement remains GH #326.

## Why vDPA / VFIO would not recreate the exact bridge limit

The VMM-facing vDPA path does not inherently create a host network interface. Linux's `vhost_vdpa` driver binds a `vdpa_device` and creates `/dev/vhost-vdpa-N`, a character device opened by the VMM. The alternative `virtio_vdpa` bus driver makes the device available to host virtio drivers and may ultimately yield a host network interface for a network device; that is a different consumer. [Linux `vhost_vdpa` probe](https://github.com/torvalds/linux/blob/v7.0/drivers/vhost/vdpa.c#L1477-L1527), [vDPA project interface split](https://vdpa-dev.gitlab.io/).

Likewise, VFIO passes a PCI function through an IOMMU. It requires no TAP or Linux bridge port. In switchdev deployments, a vendor driver may create a host representor netdevice for each VF/SF so OVS can control the embedded switch, but representors are a topology choice/vendor mechanism rather than a requirement of direct VFIO assignment.

The density problem moves to other finite resources:

- one directly assigned VF per VM NIC;
- hardware vDPA device or function/subfunction allocation;
- PCI functions/BAR space and IOMMU mappings;
- MSI-X vectors and virtqueues;
- optional representor endpoints and NIC flow-table entries;
- device/firmware-specific total counts.

The current box exposes zero such child endpoints. This result does not infer a universal vDPA or SR-IOV limit.

## Native inventory

### Platform

| Item | Observed value |
|---|---|
| Host | Dell PowerEdge C6615, hardware revision A03, system firmware 1.3.3 |
| CPU | AMD EPYC 8024P, 8 cores / 16 threads, AMD-V |
| OS | Ubuntu 26.04 LTS |
| Kernel | `7.0.0-38-generic` |
| Virtualization | `systemd-detect-virt`: `none`; canonical KVM preflight passed |
| Kernel command line | `BOOT_IMAGE=/vmlinuz-7.0.0-38-generic root=UUID=… ro console=tty1 console=ttyS0` |
| Cloud Hypervisor | `v53.0.0`, migration protocol 0 |
| vDPA tool | iproute2 `vdpa` 6.19.0 |

### NIC and management ownership

| BDF / interface | Hardware and driver | State / role | IOMMU group | SR-IOV/vDPA observation |
|---|---|---|---:|---|
| `0000:01:00.0` / `eno12399np0` | Broadcom BCM57414 10/25 GbE, `bnxt_en`; firmware `22.92.07.50`, management firmware `229.2.75.0` | **UP**, address `151.115.99.251/24`, default route: physical management/data connection | 27 | no `sriov_totalvfs`; devlink `enable_sriov=false` permanent; no vDPA device |
| `0000:01:00.1` / `eno12409np1` | second port of the same BCM57414/card and firmware | DOWN, no address | 28 | no `sriov_totalvfs`; same permanent setting false; no vDPA device |
| `0000:c7:00.0` / `eno8303` | Broadcom BCM5720 1 GbE, `tg3`; firmware `FFV22.91.5` | DOWN, no address | 16 | no SR-IOV capability, no vDPA device |

Each physical NIC function is isolated in its own IOMMU group. This proves that IOMMU grouping itself would not block full-PF assignment. It does not supply VFs or a vDPA endpoint.

The `bnxt` driver exposes `enable_sriov` only in permanent mode. In Linux 7.0 source it is stored under `BNXT_NVM_SHARED_CFG`; driver reload closes an active netdev during reinitialization, and firmware activation may reset the device. [Pinned bnxt docs](https://github.com/torvalds/linux/blob/v7.0/Documentation/networking/devlink/bnxt.rst), [`nvm_params`](https://github.com/torvalds/linux/blob/v7.0/drivers/net/ethernet/broadcom/bnxt/bnxt_devlink.c#L658-L675), [reload path](https://github.com/torvalds/linux/blob/v7.0/drivers/net/ethernet/broadcom/bnxt/bnxt_devlink.c#L425-L560).

Because the live management port and the down port are functions of the same dual-port card and the setting is stored in shared NVM, there is no proven safe “spare function” on which to enable SR-IOV. No setting, driver binding, module load, NIC reset, firmware action or reboot was attempted.

### Kernel and VMM capability

The running kernel config contains:

```text
CONFIG_PCI_IOV=y
CONFIG_VFIO=m
CONFIG_VFIO_PCI=m
CONFIG_VDPA=m
CONFIG_VDPA_SIM=m
CONFIG_VDPA_SIM_NET=m
CONFIG_VDPA_USER=m
CONFIG_IFCVF=m
CONFIG_MLX5_VDPA=y
CONFIG_ALIBABA_ENI_VDPA=m
CONFIG_SNET_VDPA=m
CONFIG_PDS_VDPA=m
CONFIG_OCTEONEP_VDPA=m
CONFIG_VHOST_VDPA=m
CONFIG_IOMMU_SUPPORT=y
CONFIG_AMD_IOMMU=y
```

All named module files were present. None was loaded by this spike. Cloud Hypervisor's actual `--help` exposed:

```text
--device path=<device_path>,fd=<vfio_cdev_fd>,iommu=on|off,...
--vdpa path=<device_path>,num_queues=<number_of_queues>,iommu=on|off,...
--vsock cid=<context_id>,...,backend=unix|vhost-kernel,...
```

This isolates the failure to a missing/excluded hardware endpoint rather than missing Cloud Hypervisor CLI support.

### vDPA state

```text
$ vdpa -V
vdpa utility, iproute2-6.19.0
$ vdpa -j mgmtdev show
Failed to connect to vdpa Netlink
$ vdpa -j dev show
Failed to connect to vdpa Netlink
$ find /sys/bus/vdpa -maxdepth 4 -print
find: '/sys/bus/vdpa': No such file or directory
```

No `/dev/vhost-vdpa-*` existed. Loading the core/simulator modules would change the physical host kernel and, more importantly, would still not create a hardware device. It was excluded.

## Open vSwitch finding

“Use Open vSwitch” is not one recommendation; it names different data paths:

1. **Kernel OVS + TAP/vhost-net.** OVS replaces the Linux bridge and removes its 1,023-port allocator obstacle. It keeps one TAP netdevice, queue FD and OVS vport per VM. This exact bounded comparison already held 16,384 endpoints in the project's stock-kernel OVS probe. It does not prove wire speed or remove the host-interface population.
2. **OVS-DPDK + vhost-user.** OVS userspace switching connects each guest virtio-net device through its own vhost-user socket/OVS port, maps hugepage-backed guest memory, and consumes PMD cores/queues. Upstream OVS recommends `dpdkvhostuserclient` over deprecated `dpdkvhostuser` because OVS and VMs can reconnect after restart. That is a recommendation *within this topology*, not a universal vDPA recommendation. [Upstream vhost-user guide](https://docs.openvswitch.org/en/latest/topics/dpdk/vhost-user/).
3. **SR-IOV switchdev + OVS representors/TC or DPDK hardware offload.** A VF goes to the guest; its representor is attached to OVS on the host; supported flow matches/actions can execute in the NIC. This avoids TAPs but consumes one VF/SF and usually one representor endpoint per guest plus hardware flow-table resources. Unsupported flows may partially offload or fall back to software. [OVS representors](https://docs.openvswitch.org/en/stable/topics/dpdk/phy/#representors), [TC flower offload](https://docs.openvswitch.org/en/latest/howto/tc-offload/).
4. **Vendor vDPA + OVS.** NVIDIA recommends VFs + switchdev + OVS in its ASAP² hardware-vDPA stack. The same vendor documentation says its OVS-DPDK vDPA API is not accepted upstream and may change. That recommendation is specific to supported ConnectX/BlueField hardware and differs from Cloud Hypervisor's kernel `/dev/vhost-vdpa-*` path. [NVIDIA ASAP² Direct](https://docs.nvidia.com/networking/display/mlnxenv587061lts/OVS%2BOffload%2BUsing%2BASAP%C2%B2%2BDirect), [NVIDIA OVS-DPDK acceleration](https://docs.nvidia.com/doca/archive/2-8-0/ovs-dpdk-hardware-acceleration.pdf).

On this host, OVS userspace tools are absent. The stock `openvswitch.ko` file is installed but not loaded, and no configured OVS-DPDK surface was found. Installing or configuring OVS would not make the current Broadcom functions expose VFs or a hardware vDPA device. No OVS change was made.

OVS flow offload supplies packet forwarding, drops, header changes, VLAN/tunnel actions and related counters. It does not by itself originate/terminate Overdrive mTLS, select an SVID, attribute a flow by launch token, or reproduce the current owner lifecycle. It can be a switching/policy control plane only after those outcomes have an approved mapping.

## Smallest real experiment selected

The read-only hardware inventory was the smallest experiment compatible with the substrate, and it decisively stopped the hardware datapath phase.

The two superficially available mutations would not answer the bounded question safely:

- **Enable SR-IOV on the BCM57414:** requires changing permanent shared NIC NVM and reinitializing/resetting the card whose first function carries management traffic. This is an outcome-changing host-device action with a credible connectivity impact. It was not authorized by “spike on metal,” and the second function is not proven independent at the firmware-reset boundary.
- **Pass through the down BCM5720 as a full PF:** it has no live link and provides one PCI endpoint, not SR-IOV/vDPA endpoint cardinality. Even a successful guest enumeration would not answer the assumption.

There is therefore no narrow host-device approval that would turn this particular hardware into a valid density spike. The useful next substrate is a dedicated non-management NIC/DPU that already exposes hardware vDPA devices or SR-IOV VFs in the running configuration.

## Commands and evidence

All physical-host reads ran through the canonical lease. The final retained
capture used this exact command:

```sh
cargo xtask metal run -- bash -c \
  'bash spike-scratch/netns-density-295-vdpa-sriov/increment-a/inventory.sh \
     > /tmp/netns-density-295-vdpa-sriov-increment-a.log; \
   spike_status=$?; \
   wc -lc /tmp/netns-density-295-vdpa-sriov-increment-a.log; \
   sha256sum /tmp/netns-density-295-vdpa-sriov-increment-a.log; \
   exit "$spike_status"'
```

The command returned:

```text
4041 236098 /tmp/netns-density-295-vdpa-sriov-increment-a.log
70e031b9a6372a868865bf859f5c2d8253671b25f6daf69821aa3d118aca95bd  /tmp/netns-density-295-vdpa-sriov-increment-a.log
```

The full inventory covered `lspci -Dnnk/-Dvv`, `/sys/class/net`, every `sriov_totalvfs`, IOMMU groups, `vdpa`, `devlink`, kernel config, `modinfo`, `lsmod`, Cloud Hypervisor help/version, addresses, routes and kernel command line.

The initial and final address, route and command-line inventories were configuration-identical. DHCP/RA lifetimes advanced normally. No interface, route, VF, vDPA device, IOMMU group, driver binding, module set, OVS datapath or firmware setting was changed.

The retained devlink excerpt is:

```text
pci/0000:01:00.0: mode legacy
  name enable_sriov type generic
    values:
      cmode permanent value false
pci/0000:01:00.1: mode legacy
  name enable_sriov type generic
    values:
      cmode permanent value false
```

## Timing

| Phase | Elapsed |
|---|---:|
| Full synced inventory, including lease/sync/preflight | 38.1 s |
| Focused NIC/SR-IOV/vDPA/devlink inventory | 33.2 s |
| Kernel config/module/PCI/IOMMU inventory | 30.0 s |
| OVS inventory | 37.0 s |
| Focused bnxt eswitch/permanent-parameter inventory | 32.7 s |
| Total canonical-run wall time | approximately 171 s |
| Final retained self-contained inventory rerun | 31.7 s |

No throughput or pps timing was possible or claimed because no hardware backend existed. This spike is separate from the GH #326 acceptance benchmark.

## Edge cases and corrected assumptions

1. **“Linux interfaces cap at 1,023.”** Incorrectly broad. One stock Linux bridge has 1,023 usable port numbers. OVS held 16,384 Linux TAP netdevices in a bounded probe.
2. **“vDPA/VFIO would recreate host TAP interfaces.”** Not inherently. `vhost-vdpa` creates a character device and VFIO assigns a PCI function. Switchdev representors may add host netdevices when OVS control is desired.
3. **“Cloud Hypervisor probably lacks vDPA.”** False. The actual v53 binary exposes `--vdpa`; the blocker is the current NIC endpoint.
4. **“A down second 25 GbE port is a safe spare.”** Unproved and unsafe to assume. It shares a dual-port BCM57414 and a shared permanent SR-IOV NVM setting with the active management port.
5. **“OVS is universally recommended for vDPA.”** False. Upstream OVS recommendations are topology-specific; NVIDIA's vDPA+OVS recipe is a vendor hardware stack and explicitly contains non-upstream API.
6. **“Software vDPA is enough to answer hardware wire speed.”** False. It would test software plumbing only.

## Design implications

- The current metal box is a measured hardware-prerequisite blocker, not evidence against vDPA/SR-IOV generally.
- vDPA/SR-IOV remain relevant to the original density motivation because they avoid the failed TAP-on-one-bridge attachment. Their endpoint/resource limits must be measured on the exact target hardware.
- OVS can remove the Linux bridge port-number ceiling, but only the hardware-offload/representor forms change the packet CPU path; all forms retain their own per-endpoint resources.
- No accepted architecture, design, roadmap, test, lane or production source changes as a result of this spike.

## Spike code

The one-file read-only inventory script and its raw output are retained, per
the repository spike rule, at:

- [`inventory.sh`](../../../../spike-scratch/netns-density-295-vdpa-sriov/increment-a/inventory.sh) — SHA-256 `dc7d29f9a134ee7a1890d7f337485920340e0a5b457a39fa75d3aceb661017a3`;
- [`inventory.raw.log.gz`](../../../../spike-scratch/netns-density-295-vdpa-sriov/increment-a/evidence/inventory.raw.log.gz) — lossless gzip, 37,471 bytes, compressed SHA-256 `83745d24f24c6c46bb83b1cec0a9ca0a9e60324fb586b9eafe6d20ac5466161f`; the decompressed 236,098 bytes have SHA-256 `70e031b9a6372a868865bf859f5c2d8253671b25f6daf69821aa3d118aca95bd`.

They are isolated from the production workspace and build graph. No file under
`crates/` was created or modified by the spike.
