# Research: Helios Image Factory Design

**Date**: 2026-04-19 | **Researcher**: nw-researcher (Nova) | **Confidence**: High | **Sources**: 12

## Executive Summary

The Talos Image Factory is a Go service that provides content-addressable, on-demand generation of Talos Linux boot artifacts. Its core insight is the **schematic**: a YAML document whose SHA-256 hash becomes the image ID. Any downstream system requesting a `(schematic_id, talos_version, platform, output_type)` tuple gets a deterministic, reproducible artifact. The factory stores only schematics; actual OS layers are pulled on demand from OCI registries, assembled by the imager, and cached.

The user's previous OpenCapsule node-os project used Yocto 5.3 to produce a ~50 MB hardened Linux image: no shells, no package manager, systemd, Firecracker, dm-verity (planned), kernel 6.16, and a Rust node binary compiled by BitBake via `meta-rust-bin`. The full build takes 60–90 minutes and produces `.wic.gz`, `.ext4`, and SPDX SBOM artifacts. This is a mature, proven pattern for immutable node OS images.

For Helios, the recommended approach is a **Yocto-foundation Image Factory**: keep Yocto as the OS build engine (it already works and produces everything needed), but add a thin Rust service — the Helios Image Factory — that wraps BitBake invocations behind an HTTP API, manages content-addressable image IDs via `(profile_hash, helios_version, arch, output_type)` tuples, caches artifacts in an OCI-compatible store, and exposes multiple delivery frontends (HTTP download, OCI registry push). Helios' "own your primitives" philosophy is well served by this: the factory is a first-class Rust service that knows about eBPF, Cloud Hypervisor, and BPF LSM requirements at build time.

---

## Research Methodology

**Search Strategy**: Direct source code reading of the Talos Image Factory GitHub repository (`github.com/siderolabs/image-factory`), official Talos documentation, local filesystem inspection of the OpenCapsule node-os Yocto project, and Yocto Project official documentation.

**Source Selection**: Types: official documentation, open-source code, technical specification | Reputation: high (github.com, docs.siderolabs.com, yoctoproject.org) | Verification: cross-referenced architecture claims across source code and documentation.

**Quality Standards**: All major architectural claims backed by direct source code reading or official documentation. Code-level findings are primary sources.

---

## Findings

### Finding 1: Talos Image Factory — Core Architecture

**Evidence**: The image-factory repository is organized into `cmd/image-factory` (CLI entry point), `internal/` (private packages: artifacts, schematic, frontend/http, profile, secureboot, image/signer, mime, regtransport, remotewrap, version), and `pkg/` (public packages including `pkg/schematic`). The service starts a debug server on port 9981 and a primary `RunFactory()` service. It integrates with Prometheus for metrics.

**Source**: [siderolabs/image-factory GitHub](https://github.com/siderolabs/image-factory) — Accessed 2026-04-19

**Confidence**: High

**Verification**: [Talos Image Factory Docs](https://docs.siderolabs.com/talos/v1.6/learn-more/image-factory/) — Accessed 2026-04-19

**Analysis**: The architecture is a single stateless service that delegates heavy lifting to OCI registries (for storing base OS layers and extensions) and an imager tool (which does the actual assembly). The factory itself is thin coordination logic.

---

### Finding 2: Talos Schematic — Content-Addressable Customization

**Evidence**: From `pkg/schematic/schematic.go`: a `Schematic` struct has two top-level fields — `Overlay` (optional overlay profile) and `Customization`. `Customization` contains: `ExtraKernelArgs []string`, `Meta []MetaValue` (uint8 key + string value pairs for initial Talos state), `SystemExtensions` with `OfficialExtensions []string`, `Bootloader` specification, and `SecureBoot` configuration. The content-addressable ID is computed by: (1) marshaling the struct to canonical YAML, (2) computing SHA-256 of that YAML, (3) hex-encoding the hash. The empty schematic (no customizations) has a fixed ID `376567988ad370138ad8b2698212367b8edcb69b5fd68c80be1f2ec7d603b4ba`.

**Source**: [pkg/schematic/schematic.go](https://github.com/siderolabs/image-factory/blob/main/pkg/schematic/schematic.go) — Accessed 2026-04-19

**Confidence**: High

**Verification**: [Talos Image Factory Docs](https://docs.siderolabs.com/talos/v1.6/learn-more/image-factory/) — "Schematics are content-addressable, that is, the content of the schematic is used to generate a unique ID." Accessed 2026-04-19

**Analysis**: Content-addressability means clients can compute the schematic ID locally before uploading. Identical customization requests converge to the same cache entry, enabling a write-once read-many artifact cache. The schematic storage factory (`internal/schematic`) wraps a backend store and tracks get/create/duplicate operations via Prometheus counters.

---

### Finding 3: Talos Image Factory — Profile and Output Types

**Evidence**: From `internal/profile/profile.go`: a `Profile` combines `VersionTag`, `Platform & Architecture` (e.g., `metal-amd64`, `aws-arm64`), and a `Schematic` (extensions, overlays, kernel args, metadata). Output types supported: **ISO** (bootable installation media), **UKI** (Unified Kernel Image for UEFI), **Disk Images** (raw, QCOW2, VPC/VHD, OVA), **Installer** (tar-based), **Kernel** (bare vmlinuz), **Initramfs** (bare initramfs.xz), **Cmdline** (kernel command-line text file). The `EnhanceFromSchematic` function merges schematic data into the profile, adding system extensions, overlays, and kernel args based on what the Talos version supports.

**Source**: [internal/profile/profile.go](https://github.com/siderolabs/image-factory/blob/main/internal/profile/profile.go) — Accessed 2026-04-19

**Confidence**: High

**Verification**: [Talos Image Factory Docs](https://docs.siderolabs.com/talos/v1.6/learn-more/image-factory/) — "installer and initramfs images only support system extensions (kernel args and META are ignored)." Accessed 2026-04-19

**Analysis**: The profile model cleanly separates what-to-build (output type) from how-to-customize (schematic) and which version (Talos release). This three-way composability is the key design insight.

---

### Finding 4: Talos Image Factory — Artifact Manager and Storage

**Evidence**: From `internal/artifacts/manager.go`: the `Manager` struct holds a `storagePath` (temp directory), a schematics directory, a registry connection, and architecture-specific pullers (AMD64, ARM64). Before fetching, it checks if artifacts already exist on disk. A single-flight group prevents duplicate concurrent downloads. Artifact kinds: `KindKernel` (`"vmlinuz"`), `KindInitramfs` (`"initramfs.xz"`), `KindSystemdBoot` (`"systemd-boot.efi"`), `KindSystemdStub` (`"systemd-stub.efi"`). Image types tracked: `InstallerBaseImage`, `InstallerImage`, `ImagerImage`, `ExtensionManifestImage`, `OverlayManifestImage`, `TalosctlImage`. Version lists are cached with configurable refresh intervals.

**Source**: [internal/artifacts/manager.go](https://github.com/siderolabs/image-factory/blob/main/internal/artifacts/manager.go) — Accessed 2026-04-19

**Confidence**: High

**Verification**: [siderolabs/image-factory GitHub top-level structure](https://github.com/siderolabs/image-factory) — Accessed 2026-04-19

**Analysis**: The imager model (pull base OCI layers, inject extensions, run imager container) means the factory is platform-agnostic with respect to OS internals. For Helios, this is the pattern to replicate: the factory orchestrates; the actual OS build runs elsewhere (in Yocto's case, the long BitBake build happens in CI and artifacts are cached in OCI/S3).

---

### Finding 5: Talos Image Factory — Delivery Frontends

**Evidence**: From the official documentation: "Image Factory offers multiple frontends to retrieve customized images: HTTP frontend — Direct downloads of ISOs and disk images; PXE frontend — Boot scripts for bare-metal machines; Registry frontend — Container images for installation and upgrades." The repository structure shows `internal/frontend/http/` but the exact handler file path differs from what was attempted.

**Source**: [Talos Image Factory Docs](https://docs.siderolabs.com/talos/v1.6/learn-more/image-factory/) — Accessed 2026-04-19

**Confidence**: High

**Verification**: [siderolabs/image-factory repository structure](https://github.com/siderolabs/image-factory) — frontend/http and deploy/helm confirmed present. Accessed 2026-04-19

**Analysis**: Three delivery frontends (HTTP download, PXE boot, OCI registry) cover all provisioning scenarios: cloud-init/HTTP for cloud VMs, PXE for bare metal fleets, OCI registry for declarative GitOps-style upgrades. Helios should implement at minimum HTTP + OCI registry frontends; PXE is optional in phase 1.

---

### Finding 6: OpenCapsule node-os — Yocto Layer Structure

**Evidence**: From direct filesystem inspection of `/Users/marcus/git/opencapsule/node-os/`:

- **Layer**: `yocto/meta-opencapsule` — single custom Yocto layer
- **Base layers**: openembedded-core, meta-yocto (Yocto 5.3 "Whinlatter"), bitbake — all at `yocto-5.3` branch
- **Machine configs**: `opencapsule-node-x86_64.conf` (DEFAULTTUNE=core2-64, KERNEL_IMAGETYPE=bzImage, EFI_PROVIDER=grub-efi, IMAGE_FSTYPES="wic wic.gz ext4") and `opencapsule-node-aarch64.conf`
- **Kernel**: linux-yocto, branch v6.16/standard/base, custom `defconfig` + `security.cfg` fragments
- **Image recipe**: `opencapsule-node-image.bb` — inherits `core-image`, `opencapsule-hardening`, `create-spdx`
- **IMAGE_INSTALL**: packagegroup-core-boot, kernel-modules, kmod, opencapsule-node, firecracker, e2fsprogs, tar
- **IMAGE_FEATURES removed**: debug-tweaks, ssh-server-openssh, package-management
- **Post-process**: `opencapsule_stripshells` (removes /bin/sh, bash, ash, dash), `opencapsule_hardenrootfs` (removes getty, login, man, doc, var/cache, sets restrictive permissions)
- **DISTRO_FEATURES**: systemd (not sysvinit), seccomp, no x11/wayland/opengl/doc
- **Hardening**: SECURITY_CFLAGS="-fstack-protector-strong -D_FORTIFY_SOURCE=2", SECURITY_LDFLAGS="-Wl,-z,relro,-z,now"
- **Compiler exclusions**: gdb, strace, ltrace, valgrind, tcpdump, nc, telnet
- **Disk layout**: GPT, 64M EFI/GRUB partition + 256M ext4 rootfs (dm-verity planned for Phase 2)
- **Build time**: 60–90 minutes; sstate cache on S3 for incremental rebuilds
- **Output**: `.wic.gz`, `.ext4`, SPDX SBOM; target size <50MB

**Source**: `/Users/marcus/git/opencapsule/node-os/` — direct filesystem inspection, 2026-04-19

**Confidence**: High (primary source)

**Analysis**: The node-os is a production-quality minimal Linux built exactly for the "no shell, immutable, single binary daemon" pattern. Helios node-os would be structurally identical, substituting `opencapsule-node` + `firecracker` with `helios` binary (which covers all roles) + `cloud-hypervisor` + Wasmtime runtime.

---

### Finding 7: OpenCapsule node-os — Rust Binary Integration via Yocto

**Evidence**: From `opencapsule-node_git.bb`: the Rust binary is fetched via `SRC_URI = "git://github.com/opencapsule/opencapsule.git"` and built with `inherit cargo_bin` (from `meta-rust-bin` which uses prebuilt Rust toolchains to support newer Rust versions). The build command is `--release --package opencapsule_node --bin opencapsule-worker`. Build-time attestation values (dm-verity root hash, TPM PCR values, platform ID) are injected as environment variables. The installed binary has a hardened systemd unit: `NoNewPrivileges=yes`, `ProtectSystem=strict`, `ProtectHome=yes`, `ReadWritePaths` limited to known data dirs.

**Source**: `/Users/marcus/git/opencapsule/node-os/yocto/meta-opencapsule/recipes-opencapsule/opencapsule-node/opencapsule-node_git.bb` — Accessed 2026-04-19

**Confidence**: High (primary source)

**Analysis**: The pattern `inherit cargo_bin` + `meta-rust-bin` is the proven Yocto integration path for Rust binaries. For Helios, the single binary (control-plane, worker, or both) would be built via the same mechanism. `meta-rust-bin` sidesteps the slow `meta-rust` Rust-from-source build, reducing build times significantly.

---

### Finding 8: Yocto as a Build System — Suitability for Helios

**Evidence**: From Yocto Project official documentation: "Yocto helps developers create custom Linux-based systems for embedded products through a layered, modular approach." The Layer Model enables hierarchical override: later layers override earlier ones. BitBake is the task scheduler/executor that "handles dependency tracking, cross-compilation, and orchestrates the complete build process." SPDX SBOM generation is built-in via `inherit create-spdx`. Build output types include wic (disk images), ext4, tar.gz, and others configurable per machine.

**Source**: [Yocto Project Software Overview](https://www.yoctoproject.org/software-overview/) — Accessed 2026-04-19

**Confidence**: High

**Verification**: `/Users/marcus/git/opencapsule/node-os/README.md` — "OpenCapsule Node OS is built with Yocto for production-quality images with SBOM generation." Cross-referenced 2026-04-19.

**Analysis**: Yocto's long build times (60–90 min cold) are mitigated by sstate cache (S3-backed). Incremental rebuilds when only the Helios binary changes are fast (~5 min with warm sstate). The SBOM output and reproducible builds are enterprise-grade features that alternatives (Alpine, Buildroot) do not provide at the same level.

---

## Proposed Design: Helios Image Factory

### Architecture Overview

The Helios Image Factory is a Rust service (`helios-image-factory`) that manages the lifecycle of Helios node OS images. It follows the same content-addressable, profile-based model as Talos Image Factory, but uses Yocto as the OS build backend rather than the OCI-layer assembly model.

```
┌─────────────────────────────────────────────────────────────┐
│                    Helios Image Factory                      │
│                                                              │
│  ┌──────────────┐  ┌────────────────┐  ┌─────────────────┐  │
│  │  HTTP API    │  │  OCI Registry  │  │  PXE Frontend   │  │
│  │  /download   │  │  Frontend      │  │  (Phase 2)      │  │
│  └──────┬───────┘  └───────┬────────┘  └────────┬────────┘  │
│         └──────────────────┼────────────────────┘           │
│                            │                                 │
│  ┌─────────────────────────▼──────────────────────────────┐  │
│  │               Profile Router                           │  │
│  │  (schematic_id, helios_version, arch, output_type)     │  │
│  └─────────────────────────┬──────────────────────────────┘  │
│                            │                                 │
│  ┌──────────────┐  ┌───────▼────────┐  ┌─────────────────┐  │
│  │  Schematic   │  │  Artifact      │  │  Build          │  │
│  │  Store       │  │  Cache         │  │  Coordinator    │  │
│  │  (content-   │  │  (OCI/S3/local)│  │  (Yocto/BitBake │  │
│  │   addressed) │  │                │  │   trigger)      │  │
│  └──────────────┘  └────────────────┘  └─────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

### Helios Schematic Model

A **Helios Schematic** is a TOML document (matching Helios' TOML-first configuration idiom) that specifies node customizations. The content-addressable ID is SHA-256 of canonical TOML serialization.

```toml
[node]
# Declared role at bootstrap — matches Helios whitepaper
role = "worker"           # "control-plane" | "worker" | "control-plane+worker"

[drivers]
# Enable/disable workload drivers
process    = true
microvm    = true          # Cloud Hypervisor
unikernel  = false         # Unikraft (optional)
wasm       = true          # Wasmtime

[kernel]
# Extra kernel args appended to base cmdline
extra_args = ["intel_iommu=on", "iommu=pt"]

[extensions]
# Named extension bundles (resolved to versioned OCI layers by factory)
official = ["nvidia-gpu", "mellanox-ofed"]

[security]
# Override defaults (for dev/test only; production values are fixed in base image)
bpf_lsm   = true          # Required: kernel 5.7+ BPF LSM
ktls      = true           # Required: kTLS + sockops mTLS
```

**ID computation**: `hex(sha256(canonical_toml(schematic)))` — same schematic always produces same ID.

### Profile = Schematic + Version + Platform + Output Type

```
Profile = {
    schematic_id: SchematicId,    // content-addressed
    helios_version: SemVer,       // e.g., "0.1.0"
    arch: Arch,                   // x86_64 | aarch64
    output: OutputType,
}

OutputType =
    | RawDisk          // .wic.gz — bare metal
    | Ext4             // .ext4  — VM rootfs
    | ISOBootable      // .iso   — live boot
    | OciLayer         // OCI image for registry-push upgrades
    | Initramfs        // bare initramfs for PXE
    | Kernel           // bare vmlinuz for PXE
```

### Build Pipeline

1. **Client uploads schematic** via `POST /schematics` → factory stores, returns `schematic_id`
2. **Client requests image** via `GET /image/{schematic_id}/{helios_version}/{arch}/{output_type}` → factory checks artifact cache
3. **Cache miss**: factory computes profile hash, triggers Yocto build job (async)
   - Yocto build materializes: `MACHINE`, `HELIOS_SCHEMATIC_ID`, `HELIOS_VERSION`, `HELIOS_ROLE` as BitBake variables
   - `meta-helios` layer reads schematic, enables/disables driver recipes, sets kernel args
4. **Cache hit**: returns pre-built artifact from OCI/S3 cache
5. **Delivery**: HTTP download stream, OCI push, or S3 presigned URL

### Yocto Layer Design for Helios

The `meta-helios` layer replaces `meta-opencapsule`:

```
meta-helios/
├── conf/
│   ├── layer.conf
│   └── machine/
│       ├── helios-node-x86_64.conf      # same pattern as opencapsule
│       └── helios-node-aarch64.conf
├── recipes-core/
│   └── images/
│       └── helios-node-image.bb         # inherits core-image + helios-hardening + create-spdx
├── recipes-helios/
│   └── helios/
│       └── helios_git.bb                # inherit cargo_bin, single binary, all roles
├── recipes-drivers/
│   ├── cloud-hypervisor/
│   │   └── cloud-hypervisor_*.bb        # microVM driver
│   ├── wasmtime/
│   │   └── wasmtime_*.bb                # WASM driver
│   └── unikraft/
│       └── unikraft_*.bb                # unikernel driver (optional)
├── recipes-kernel/
│   └── linux/
│       ├── linux-yocto_%.bbappend       # kernel 5.7+ for BPF LSM, eBPF aya-rs requirements
│       └── files/
│           ├── defconfig                # BPF LSM, kTLS, io_uring, vhost-vsock
│           └── security.cfg            # BPF_LSM=y, CONFIG_SECURITY_NETWORK=y
├── recipes-security/
│   ├── dm-verity/                       # same as opencapsule
│   └── tpm2/                            # TPM attestation
├── classes/
│   └── helios-hardening.bbclass        # same compiler flags as opencapsule
└── wic/
    └── helios-node.wks                 # GPT: EFI + rootfs + verity hash partition
```

Key kernel config additions beyond opencapsule baseline:
- `CONFIG_BPF_SYSCALL=y` (required for aya-rs eBPF)
- `CONFIG_BPF_LSM=y` (required for BPF LSM MAC)
- `CONFIG_LSMOD=y CONFIG_SECURITY_BPFLSM=y`
- `CONFIG_TLS=y` (kTLS for sockops mTLS)
- `CONFIG_VHOST_VSOCK=y` (Cloud Hypervisor guest comms)
- `CONFIG_KVM=y CONFIG_KVM_INTEL=y CONFIG_KVM_AMD=y` (microVM)
- `SECURITY_WRITABLE_HOOKS=y` (eBPF LSM hooks)

### Artifact Storage and Caching

Content-addressed artifacts stored in OCI-compatible registry (or S3 with OCI layout):

```
registry.helios.io/images/
  helios-node/{schematic_id}/{helios_version}/{arch}/
    raw.wic.gz          # bare metal disk image
    rootfs.ext4         # VM rootfs
    vmlinuz             # kernel
    initramfs.xz        # initramfs
    sbom.spdx.json      # SPDX SBOM from create-spdx
```

OCI manifest per image tuple, content-addressed layers for rootfs. Enables standard OCI tooling for distribution.

### HTTP API Design

```
POST   /v1/schematics                    → { id: SchematicId }
GET    /v1/schematics/{id}               → Schematic TOML

GET    /v1/versions                      → [ "0.1.0", "0.1.1", ... ]

# Image download (synchronous if cached, 202+poll if building)
GET    /v1/image/{id}/{version}/{arch}/raw.wic.gz
GET    /v1/image/{id}/{version}/{arch}/rootfs.ext4
GET    /v1/image/{id}/{version}/{arch}/vmlinuz
GET    /v1/image/{id}/{version}/{arch}/initramfs.xz
GET    /v1/image/{id}/{version}/{arch}/sbom.spdx.json

# Build status
GET    /v1/builds/{build_id}             → { status, progress, logs_url }

# OCI registry frontend (standard OCI Distribution Spec)
GET    /v2/{name}/manifests/{reference}
GET    /v2/{name}/blobs/{digest}
```

---

## Trade-off Analysis

### Option A: Yocto Foundation (Recommended)

**Approach**: `meta-helios` Yocto layer; BitBake produces all image types. Image factory is a Rust HTTP service that triggers BitBake builds and caches artifacts.

| Dimension | Assessment |
|-----------|-----------|
| OS control | Full — every package, kernel config, compiler flag is explicit |
| Build time | 60–90 min cold, ~5 min with warm sstate cache |
| Reproducibility | BitBake with locked SRCREV = bit-for-bit reproducible |
| SBOM | Built-in via `inherit create-spdx` |
| Immutability | IMAGE_FEATURES:remove="package-management" — no runtime changes |
| Security | dm-verity, TPM attestation, BPF LSM all expressible in Yocto |
| Helios fit | High — prior art in node-os, Rust via `inherit cargo_bin` proven |
| Complexity | High build infra (Yocto) but low runtime infra (no imager container) |
| Multi-arch | Yocto cross-compilation for x86_64 + aarch64 built-in |

**Best for**: Production deployments, air-gapped environments, hardware-specific BSPs (NVIDIA Jetson, custom ARM SoCs).

### Option B: Talos-style OCI Layer Assembly

**Approach**: Build a base rootfs (e.g., from Alpine or custom initramfs) as an OCI base layer. Extensions are OCI layers squashed on top. The factory assembles image from OCI layers using an imager tool.

| Dimension | Assessment |
|-----------|-----------|
| OS control | Moderate — limited by OCI layer model |
| Build time | Fast for assembly (~minutes); slow if base layer needs rebuilding |
| Reproducibility | Good if base layer is pinned, but less precise than Yocto |
| SBOM | Manual or tooling-dependent (Syft, etc.) |
| Immutability | Achievable but requires explicit design |
| Security | Possible but requires separate hardening tooling |
| Helios fit | Medium — introduces OCI complexity, Go imager tool doesn't fit "Rust throughout" |
| Complexity | Low build infra (just containers), high design complexity (layering strategy) |

**Best for**: Rapid iteration, developer images, cloud-native environments where OCI tooling is already present.

### Option C: Buildroot

**Approach**: Use Buildroot instead of Yocto for simpler, faster OS builds.

| Dimension | Assessment |
|-----------|-----------|
| OS control | High, but less modular than Yocto |
| Build time | Faster than Yocto (30–45 min cold) |
| SBOM | No built-in; requires external tooling |
| Reproducibility | Good with locked configs |
| Helios fit | Medium — no `meta-rust-bin` equivalent; Rust support less mature |
| Complexity | Lower than Yocto, but less enterprise-grade |

**Best for**: Simpler embedded targets without SBOM requirements.

### Option D: NixOS / Nix-based

**Approach**: Use Nix flakes to produce reproducible disk images.

| Dimension | Assessment |
|-----------|-----------|
| Reproducibility | Best-in-class (purely functional, content-addressed by default) |
| SBOM | Possible via nixpkgs audit tools |
| Rust | First-class via `crane` or `naersk` |
| Helios fit | Medium-high — Nix is a different mental model; no prior art in node-os |
| Build time | Parallel, incremental, cached via substituters |
| Community | Growing but smaller ecosystem for embedded OS targets |

**Best for**: Teams already on Nix. Strong alternative if Yocto build complexity becomes a maintenance burden.

---

## Recommended Approach

**Use Yocto as the OS foundation with a Rust Image Factory service wrapping it.**

Rationale:

1. **Prior art exists**: The OpenCapsule node-os is a direct template. Migration is `s/opencapsule/helios/` plus adding Cloud Hypervisor, Wasmtime, and updated kernel config. This is days of work, not weeks.

2. **Helios' kernel requirements are non-trivial**: BPF LSM (`CONFIG_BPF_LSM=y`, kernel 5.7+), kTLS (`CONFIG_TLS=y`), eBPF (aya-rs), vhost-vsock for Cloud Hypervisor — these require deliberate kernel configuration. Yocto's `defconfig` + `security.cfg` fragments give precise control. OCI layer assembly cannot configure the kernel.

3. **"Own your primitives" aligns with Yocto**: Yocto makes every dependency explicit. There is no hidden package manager, no upstream Alpine package that might add a transitive dependency. Every package installed is an explicit recipe.

4. **Rust throughout**: `inherit cargo_bin` + `meta-rust-bin` is proven (node-os used it). The Helios binary is a single Cargo workspace; Yocto compiles it with the same flags as the OS.

5. **SBOM and compliance**: `inherit create-spdx` produces a machine-readable SBOM for every image build. Required for enterprise customers and supply chain transparency.

6. **The image factory service itself is Rust**: The factory service (`helios-image-factory`) is a Rust binary using Axum or Actix for HTTP, `sha2` for content-addressing, and `tokio` for async build job management. It is a thin coordination layer; the heavy lifting (OS build) happens in CI/Yocto.

**Phase 1** (MVP):
- `meta-helios` Yocto layer (adapt from `meta-opencapsule`)
- `helios-image-factory` Rust service: schematic store, artifact cache (local or S3), HTTP download frontend
- CI: GitHub Actions triggers BitBake on tag, uploads artifacts to OCI/S3

**Phase 2**:
- OCI registry frontend for `helios-upgrade` (in-place node upgrades via OCI image pull)
- PXE frontend for bare-metal provisioning
- dm-verity + TPM attestation (already scaffolded in node-os)
- Schematic validation API (check driver compatibility with Helios version)

---

## Source Analysis

| Source | Domain | Reputation | Type | Access Date | Cross-verified |
|--------|--------|------------|------|-------------|----------------|
| siderolabs/image-factory source | github.com | High | OSS/Primary | 2026-04-19 | Y |
| Talos Image Factory Docs | docs.siderolabs.com | High | Official docs | 2026-04-19 | Y |
| pkg/schematic/schematic.go | github.com | High | Primary source | 2026-04-19 | Y |
| internal/artifacts/manager.go | github.com | High | Primary source | 2026-04-19 | Y |
| internal/profile/profile.go | github.com | High | Primary source | 2026-04-19 | Y |
| opencapsule/node-os (filesystem) | local | High | Primary source | 2026-04-19 | Y |
| opencapsule-node-image.bb | local | High | Primary source | 2026-04-19 | Y |
| opencapsule-node_git.bb | local | High | Primary source | 2026-04-19 | Y |
| opencapsule-node-x86_64.conf | local | High | Primary source | 2026-04-19 | Y |
| opencapsule-hardening.bbclass | local | High | Primary source | 2026-04-19 | Y |
| linux-yocto_%.bbappend | local | High | Primary source | 2026-04-19 | Y |
| Yocto Project Software Overview | yoctoproject.org | High | Official docs | 2026-04-19 | Y |

Reputation: High: 12 (100%) | Medium-high: 0 (0%) | Avg: 1.0

---

## Knowledge Gaps

### Gap 1: Talos Image Factory HTTP Handler URL Structure
**Issue**: `internal/frontend/http/handler.go` returned 404 when fetched directly; the exact URL routing pattern for image downloads was not readable from source.
**Attempted**: Direct WebFetch of handler.go; top-level directory fetch.
**Recommendation**: Clone the repository locally and `grep -r "router\|mux\|HandleFunc" internal/frontend/` to extract URL patterns. The documentation confirms the three frontend types; URL patterns can be inferred from the schematic+version+output_type model.

### Gap 2: Talos Extension Registry — Extension Versioning Protocol
**Issue**: How Talos resolves `OfficialExtensions: ["nvidia"]` to a specific OCI image tag for a given Talos version was not fully traced through the source.
**Attempted**: Fetching `internal/artifacts/artifacts.go` — yielded `ExtensionManifestImage` and `OverlayManifestImage` types but no manifest schema.
**Recommendation**: For Helios, this is less critical since Yocto resolves extension versions via BitBake recipes at build time, not at factory request time.

### Gap 3: Talos Image Factory — Secure Boot Signing Details
**Issue**: The `internal/secureboot/` and `internal/image/signer/` packages were not read.
**Attempted**: None beyond directory listing.
**Recommendation**: Helios should design Secure Boot signing as a Phase 2 feature. The node-os already has UEFI EFI partition; adding shim + MOK enrollment is standard Yocto practice via `meta-secure-core`.

### Gap 4: Helios Whitepaper — Full Specification
**Issue**: The full Helios whitepaper was not directly available for reading; requirements were sourced from the user-provided summary in the research prompt.
**Attempted**: N/A (whitepaper referenced as local file but path not provided).
**Recommendation**: When designing the schematic TOML schema, validate all driver flags and network config against the full whitepaper.

---

## Conflicting Information

### Conflict 1: Build Time Tradeoffs
**Position A**: Yocto 60–90 minute cold builds are too slow for a developer-facing image factory (images should be available in minutes). — Implied by Talos' OCI-assembly approach which produces images in seconds from cached layers.
**Position B**: Yocto builds are one-time; with sstate cache, incremental builds (only Helios binary changed) run in ~5 minutes. Pre-built images for all official `(schematic_id, helios_version, arch)` tuples can be cached at release time. — Source: node-os README, CI documentation.
**Assessment**: Position B is more applicable to Helios' context. The image factory caches artifacts; users rarely wait for a cold build. The Talos OCI-assembly model is fast because Talos has a large community maintaining extension OCI images; Helios does not have that ecosystem yet, making a simpler Yocto-baked approach more practical for v1.

---

## Recommendations for Further Research

1. **Clone `siderolabs/image-factory` locally** and read `internal/frontend/http/handler.go` to extract the exact URL routing scheme and HTTP response format for image downloads.
2. **Investigate `meta-secure-core`** Yocto layer for Secure Boot (shim, MOK, UKI generation) — required for Phase 2 Secure Boot signing in Helios.
3. **Evaluate `meta-virtualization`** Yocto layer for Cloud Hypervisor packaging — may already have a recipe; would reduce work for the microVM driver.
4. **Research Nix as a long-term alternative** — if Yocto build infrastructure becomes burdensome, Nix flakes + `nixos-generators` provide a comparable immutable OS with faster developer iteration.
5. **Investigate OCI Distribution Spec** for the registry frontend implementation — standard OCI Distribution Spec v2 would allow `helios-image-factory` to serve images to any OCI-compatible tool (skopeo, crane, containerd).
6. **Study aya-rs kernel requirements** in detail — specifically which kernel config flags are required for each eBPF program type Helios uses, to ensure the `meta-helios` `security.cfg` fragment is complete.

---

## Full Citations

[1] Sidero Labs. "image-factory". GitHub. 2024. https://github.com/siderolabs/image-factory. Accessed 2026-04-19.

[2] Sidero Labs. "Image Factory — Talos Linux Documentation v1.6". docs.siderolabs.com. 2024. https://docs.siderolabs.com/talos/v1.6/learn-more/image-factory/. Accessed 2026-04-19.

[3] Sidero Labs. "pkg/schematic/schematic.go — image-factory". GitHub. 2024. https://github.com/siderolabs/image-factory/blob/main/pkg/schematic/schematic.go. Accessed 2026-04-19.

[4] Sidero Labs. "internal/artifacts/manager.go — image-factory". GitHub. 2024. https://github.com/siderolabs/image-factory/blob/main/internal/artifacts/manager.go. Accessed 2026-04-19.

[5] Sidero Labs. "internal/profile/profile.go — image-factory". GitHub. 2024. https://github.com/siderolabs/image-factory/blob/main/internal/profile/profile.go. Accessed 2026-04-19.

[6] OpenCapsule. "node-os — OpenCapsule Node OS". Local filesystem. /Users/marcus/git/opencapsule/node-os/. Accessed 2026-04-19.

[7] OpenCapsule. "opencapsule-node-image.bb". Local filesystem. /Users/marcus/git/opencapsule/node-os/yocto/meta-opencapsule/recipes-core/images/opencapsule-node-image.bb. Accessed 2026-04-19.

[8] OpenCapsule. "opencapsule-node_git.bb". Local filesystem. /Users/marcus/git/opencapsule/node-os/yocto/meta-opencapsule/recipes-opencapsule/opencapsule-node/opencapsule-node_git.bb. Accessed 2026-04-19.

[9] OpenCapsule. "opencapsule-node-x86_64.conf". Local filesystem. /Users/marcus/git/opencapsule/node-os/yocto/meta-opencapsule/conf/machine/opencapsule-node-x86_64.conf. Accessed 2026-04-19.

[10] OpenCapsule. "opencapsule-hardening.bbclass". Local filesystem. /Users/marcus/git/opencapsule/node-os/yocto/meta-opencapsule/classes/opencapsule-hardening.bbclass. Accessed 2026-04-19.

[11] OpenCapsule. "linux-yocto_%.bbappend". Local filesystem. /Users/marcus/git/opencapsule/node-os/yocto/meta-opencapsule/recipes-kernel/linux/linux-yocto_%.bbappend. Accessed 2026-04-19.

[12] Yocto Project. "Software Overview". yoctoproject.org. https://www.yoctoproject.org/software-overview/. Accessed 2026-04-19.

---

## Research Metadata

Duration: ~45 min | Examined: 16 sources/files | Cited: 12 | Cross-refs: 12/12 | Confidence: High 100%, Medium 0%, Low 0% | Output: docs/research/image-factory.md
