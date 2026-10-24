#!/usr/bin/env bash
# SPIKE: netns-density-295 vDPA/SR-IOV native feasibility inventory.
# Read-only on the physical host. No module, NIC, VF, firmware, OVS, or IOMMU mutation.
set -u

section() { printf '\n===== %s =====\n' "$1"; }
run() {
  printf '\n$'
  printf ' %q' "$@"
  printf '\n'
  "$@" 2>&1 || printf '[exit=%s]\n' "$?"
}

section identity
run date --iso-8601=seconds
run uname -a
run systemd-detect-virt
run lscpu
run cat /proc/cmdline
run hostnamectl

section management-network
printf 'SSH_CONNECTION=%s\n' "${SSH_CONNECTION-}"
run ip -j address show
run ip -j route show table all
if [ -n "${SSH_CONNECTION-}" ]; then
  client_ip=${SSH_CONNECTION%% *}
  run ip -j route get "$client_ip"
fi

section pci-network
run lspci -Dnnk
run lspci -Dvvnn

section netdevs
for net_path in /sys/class/net/*; do
  [ -e "$net_path" ] || continue
  net_name=${net_path##*/}
  printf '\n--- netdev %s ---\n' "$net_name"
  for field in ifindex iflink address operstate mtu phys_port_name phys_switch_id; do
    if [ -r "$net_path/$field" ]; then
      printf '%s=' "$field"
      run cat "$net_path/$field"
    fi
  done
  if [ -L "$net_path/device" ]; then
    device_path=$(readlink -f "$net_path/device")
    printf 'device=%s\n' "$device_path"
    if [ -L "$device_path/driver" ]; then
      printf 'driver=%s\n' "$(basename "$(readlink -f "$device_path/driver")")"
    fi
    for field in vendor device subsystem_vendor subsystem_device sriov_totalvfs sriov_numvfs; do
      if [ -r "$device_path/$field" ]; then
        printf '%s=' "$field"
        run cat "$device_path/$field"
      fi
    done
    if [ -L "$device_path/iommu_group" ]; then
      printf 'iommu_group=%s\n' "$(basename "$(readlink -f "$device_path/iommu_group")")"
    else
      printf 'iommu_group=NONE\n'
    fi
  fi
  command -v ethtool >/dev/null 2>&1 && run ethtool -i "$net_name"
done

section sriov-functions
for total_path in /sys/bus/pci/devices/*/sriov_totalvfs; do
  [ -e "$total_path" ] || continue
  pci_path=${total_path%/sriov_totalvfs}
  bdf=${pci_path##*/}
  printf '\n--- SR-IOV PF %s ---\n' "$bdf"
  run cat "$total_path"
  run cat "$pci_path/sriov_numvfs"
  [ -L "$pci_path/driver" ] && printf 'driver=%s\n' "$(basename "$(readlink -f "$pci_path/driver")")"
  [ -L "$pci_path/iommu_group" ] && printf 'iommu_group=%s\n' "$(basename "$(readlink -f "$pci_path/iommu_group")")"
done

section iommu-groups
if [ -d /sys/kernel/iommu_groups ]; then
  run find /sys/kernel/iommu_groups -maxdepth 3 -type l -printf '%p -> %l\n'
else
  printf 'NO /sys/kernel/iommu_groups\n'
fi
run dmesg --level=err,warn

section vdpa
if command -v vdpa >/dev/null 2>&1; then
  run vdpa -V
  run vdpa -j mgmtdev show
  run vdpa -j dev show
else
  printf 'vdpa tool: ABSENT\n'
fi
run find /sys/bus/vdpa -maxdepth 4 -print
run find /dev -maxdepth 1 -name 'vhost-vdpa*' -ls

section devlink
if command -v devlink >/dev/null 2>&1; then
  run devlink -j dev show
  run devlink -j dev info
  run devlink -j port show
  run devlink -j dev eswitch show
  for dev in pci/0000:01:00.0 pci/0000:01:00.1; do
    run devlink dev eswitch show "$dev"
    run devlink dev param show "$dev"
  done
else
  printf 'devlink tool: ABSENT\n'
fi

section kernel-config
kernel_config=/boot/config-$(uname -r)
if [ -r "$kernel_config" ]; then
  run grep -E '^(CONFIG_(PCI_IOV|IOMMU_SUPPORT|AMD_IOMMU|INTEL_IOMMU|VFIO|VFIO_PCI|VDPA|VHOST_VDPA|VDPA_SIM|VDPA_SIM_NET|VDPA_USER|IFCVF|MLX5_VDPA|PDS_VDPA|OCTEONEP_VDPA|SNET_VDPA|ALIBABA_ENI_VDPA)=|# CONFIG_(VFIO|VDPA|VHOST_VDPA|IFCVF|MLX5_VDPA|PDS_VDPA|OCTEONEP_VDPA|SNET_VDPA|ALIBABA_ENI_VDPA) is not set)' "$kernel_config"
else
  printf 'kernel config unreadable: %s\n' "$kernel_config"
fi

section module-availability
for module in vhost_vdpa vdpa virtio_vdpa vdpa_sim vdpa_sim_net ifcvf mlx5_vdpa pds_vdpa octep_vdpa snet_vdpa vfio_pci; do
  run modinfo "$module"
done
run lsmod

section open-vswitch
for command_name in ovs-vsctl ovs-vswitchd ovs-ofctl ovs-appctl ovs-dpctl; do
  printf '%s=' "$command_name"
  command -v "$command_name" || true
done
run modinfo openvswitch
run lsmod

section cloud-hypervisor
if command -v cloud-hypervisor >/dev/null 2>&1; then
  run cloud-hypervisor --version
  run cloud-hypervisor --help
else
  printf 'cloud-hypervisor on PATH: ABSENT\n'
fi
for binary in \
  /srv/vm/overdrive-testing/cloud-hypervisor \
  /home/ubuntu/overdrive/vendors/cloud-hypervisor/target/release/cloud-hypervisor \
  /home/ubuntu/overdrive/target/release/cloud-hypervisor; do
  if [ -x "$binary" ]; then
    run "$binary" --version
    run "$binary" --help
  fi
done

section final-state
run ip -j address show
run ip -j route show table all
if [ -n "${SSH_CONNECTION-}" ]; then
  client_ip=${SSH_CONNECTION%% *}
  run ip -j route get "$client_ip"
fi
run cat /proc/cmdline
