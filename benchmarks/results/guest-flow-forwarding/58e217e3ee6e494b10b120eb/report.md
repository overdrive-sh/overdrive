# Benchmark report: guest-flow-forwarding / guest_flow_forwarding

- run id: 58e217e3ee6e494b10b120eb
- family: guest-flow-forwarding
- benchmark: guest_flow_forwarding
- evidence: maintained
- substrate: guest
- commit: 36618cba7a86588dce3608ed3f898890017b9ef4
- dirty: true
- methodology: 1
- seed: 2971026034431741928
- debug_assertions: false
- kernel: 7.0.0-38-generic
- cpu_model: AMD EPYC 8024P 8-Core Processor
- cpu_count: 16
- governor: schedutil
- smt: on
- boost: on
- bpf_stats_enabled: 0
- clocksource: tsc
- irq_time_accounting: n
- guest launch: metal host (not nested)
- guest boot: fresh guest per replicate
- guest vcpu scheduling: unpinned
- guest governor: absent: not visible from a guest
- guest smt: absent: not visible from a guest
- guest boost: absent: not visible from a guest
- guest vcpus: 2
- guest memory mib: 512
- guest kernel: 7.0.0-38-generic
- guest clocksource: tsc
- guest artifact overdrive_init: 010f5819f7876bf7a64db6f4d0f8ba2c02d8946d289acb05042ce33d60db63d0
- guest artifact overdrive_guest_peer: 537b159757b75fba3f716767772d4974f021af36cb13068908025178014dcfaa
- guest artifact ovd_fwd: 0e64db0873bffd666da6a98588b06ce2e0fa5fe88b49920d64a0a9a34615fca1
- bpf stats: off
- clock: host CLOCK_MONOTONIC_RAW; service none; guest CLOCK_MONOTONIC (in-guest Instant, 1 ns)
- canary: steady
- replicates ok: 29
- replicates failed: 1
- status: finished
## Profile: default

### Table: forwarded-path

| metric | arm | p50 | p90 | p95 | p99 | values | replicates | failures |
|---|---|---|---|---|---|---|---|---|
| forwarded_round_trip_ns | forwarding | 51335.4109693542 | 57681.12561105208 | 62172.687937743176 | 72775.03462184878 | 145000 | 29 | 1 |

percentiles of forwarded_round_trip_ns are histogram-derived (relative error at most 3.125% for values of 1 and above, absolute error below 1 for smaller values)

## Failures

- profile default block 10 position 0 arm forwarding: guest run: staging the guest image: guest program: the guest sent no beacon within 60s; console:
,launch=6c616e652d6c61756e63682d746f6b65,beacon=1053110598,stream=1053110599,control=1053110600
[    0.003430] random: crng init done
[    0.003431] printk: log buffer data + meta data: 262144 + 1114112 = 1376256 bytes
[    0.003450] Dentry cache hash table entries: 65536 (order: 7, 524288 bytes, linear)
[    0.003460] Inode-cache hash table entries: 32768 (order: 6, 262144 bytes, linear)
[    0.003494] Fallback order for Node 0: 0 
[    0.003497] Built 1 zonelists, mobility grouping on.  Total pages: 130975
[    0.003498] Policy zone: DMA32
[    0.003505] mem auto-init: stack:all(zero), heap alloc:on, heap free:off
[    0.005081] SLUB: HWalign=64, Order=0-3, MinObjects=0, CPUs=2, Nodes=1
[    0.018827] ftrace: allocating 63997 entries in 252 pages
[    0.018830] ftrace: allocated 252 pages with 6 groups
[    0.019688] Dynamic Preempt: lazy
[    0.019815] rcu: Preemptible hierarchical RCU implementation.
[    0.019816] rcu: 	RCU restricting CPUs from NR_CPUS=8192 to nr_cpu_ids=2.
[    0.019818] 	Trampoline variant of Tasks RCU enabled.
[    0.019818] 	Rude variant of Tasks RCU enabled.
[    0.019819] 	Tracing variant of Tasks RCU enabled.
[    0.019820] rcu: RCU calculated value of scheduler-enlistment delay is 100 jiffies.
[    0.019821] rcu: Adjusting geometry for rcu_fanout_leaf=16, nr_cpu_ids=2
[    0.019832] RCU Tasks: Setting shift to 1 and lim to 1 rcu_task_cb_adjust=1 rcu_task_cpu_ids=2.
[    0.019833] RCU Tasks Rude: Setting shift to 1 and lim to 1 rcu_task_cb_adjust=1 rcu_task_cpu_ids=2.
[    0.025003] NR_IRQS: 524544, nr_irqs: 424, preallocated irqs: 0
[    0.025057] rcu: srcu_init: Setting srcu_struct sizes based on contention.
[    0.025067] clocksource: jiffies: mask: 0xffffffff max_cycles: 0xffffffff, max_idle_ns: 1911260446275000 ns
[    0.025136] Console: colour dummy device 80x25
[    0.025228] printk: console [ttyS0] enabled
[    0.202588] ACPI: Core revision 20251212
[    0.203673] APIC: Switch to symmetric I/O mode setup
[    0.206140] x2apic enabled
[    0.208075] APIC: Switched APIC routing to: physical x2apic
[    0.209527] kvm-guest: APIC: send_IPI_mask() replaced with kvm_send_ipi_mask()
[    0.211391] kvm-guest: APIC: send_IPI_mask_allbutself() replaced with kvm_send_ipi_mask_allbutself()
[    0.213736] kvm-guest: setup PV IPIs
[    0.214741] clocksource: tsc-early: mask: 0xffffffffffffffff max_cycles: 0x228aeee0e7a, max_idle_ns: 440795314296 ns
[    0.217439] Calibrating delay loop (skipped) preset value.. 4792.80 BogoMIPS (lpj=2396400)
[    0.218434] x86/cpu: User Mode Instruction Prevention (UMIP) activated
[    0.218434] Last level iTLB entries: 4KB 512, 2MB 512, 4MB 256
[    0.218434] Last level dTLB entries: 4KB 3072, 2MB 3072, 4MB 1536, 1GB 0
[    0.218434] mitigations: Enabled attack vectors: user_kernel, user_user, guest_host, guest_guest, SMT mitigations: auto
[    0.218434] Speculative Store Bypass: Mitigation: Speculative Store Bypass disabled via prctl
[    0.218434] Transient Scheduler Attacks: Vulnerable: No microcode
[    0.218434] Spectre V2 : Mitigation: Enhanced / Automatic IBRS
[    0.218434] Speculative Return Stack Overflow: Mitigation: Safe RET
[    0.218434] Spectre V1 : Mitigation: usercopy/swapgs barriers and __user pointer sanitization
[    0.218434] Spectre V2 : Enabling IBPB for BPF
[    0.218434] Spectre V2 : mitigation: Enabling conditional Indirect Branch Prediction Barrier
[    0.218434] active return thunk: srso_alias_return_thunk
[    0.218434] x86/fpu: Supporting XSAVE feature 0x001: 'x87 floating point registers'
[    0.218434] x86/fpu: Supporting XSAVE feature 0x002: 'SSE registers'
[    0.218434] x86/fpu: Supporting XSAVE feature 0x004: 'AVX registers'
[    0.218434] x86/fpu: Supporting XSAVE feature 0x020: 'AVX-512 opmask'
[    0.218434] x86/fpu: Supporting XSAVE feature 0x040: 'AVX-512 Hi256'
[    0.218434] x86/fpu: Supporting XSAVE feature 0x080: 'AVX-512 ZMM_Hi256'
[    0.218434] x86/fpu: Supporting XSAVE feature 0x200: 'Protection Keys User registers'
[    0.218434] x86/fpu: Supporting XSAVE feature 0x800: 'Control-flow User registers'
[    0.218434] x86/fpu: Supporting XSAVE feature 0x1000: 'Control-flow Kernel registers (KVM only)'
[    0.218434] x86/fpu: xstate_offset[2]:  576, xstate_sizes[2]:  256
[    0.218434] x86/fpu: xstate_offset[5]:  832, xstate_sizes[5]:   64
[    0.218434] x86/fpu: xstate_offset[6]:  896, xstate_sizes[6]:  512
[    0.218434] x86/fpu: xstate_offset[7]: 1408, xstate_sizes[7]: 1024
[    0.218434] x86/fpu: xstate_offset[9]: 2432, xstate_sizes[9]:    8
[    0.218434] x86/fpu: xstate_offset[11]: 2440, xstate_sizes[11]:   16
[    0.218434] x86/fpu: xstate_offset[12]: 2456, xstate_sizes[12]:   24
[    0.218434] x86/fpu: Enabled xstate features 0x1ae7, context size is 2480 bytes, using 'compacted' format.
[    0.218434] Freeing SMP alternatives memory: 56K
[    0.218434] pid_max: default: 32768 minimum: 301
[    0.218434] landlock: Up and running.
[    0.218434] Yama: becoming mindful.
[    0.218434] AppArmor: AppArmor initialized
[    0.218434] AppArmor: AppArmor secmark mediation reserved: ready to be enabled
[    0.218434] Mount-cache hash table entries: 1024 (order: 1, 8192 bytes, linear)
[    0.218434] Mountpoint-cache hash table entries: 1024 (order: 1, 8192 bytes, linear)
[    0.218434] VFS: Finished mounting rootfs on nullfs
[    0.218434] smpboot: CPU0: AMD EPYC 8024P 8-Core Processor (family: 0x19, model: 0xa0, stepping: 0x2)
[    0.218712] Performance Events: Fam17h+ core perfctr, AMD PMU driver.
[    0.219451] ... version:                   2
[    0.220439] ... bit width:                 48
[    0.221439] ... generic counters:          6
[    0.222440] ... generic bitmap:            000000000000003f
[    0.223440] ... fixed-purpose counters:    0
[    0.224439] ... fixed-purpose bitmap:      0000000000000000
[    0.225439] ... value mask:                0000ffffffffffff
[    0.226439] ... max period:                00007fffffffffff
[    0.227439] ... global_ctrl mask:          000000000000003f
[    0.228569] signal: max sigframe size: 3376
[    0.229517] rcu: Hierarchical SRCU implementation.
[    0.229519] rcu: 	Max phase no-delay instances is 400.
[    0.231539] Timer migration: 1 hierarchy levels; 8 children per group; 1 crossnode level
[    0.240338] smp: Bringing up secondary CPUs ...
[    0.240631] smpboot: x86: Booting SMP configuration:
[    0.240635] .... node  #0, CPUs:      #1
[    0.241508] smp: Brought up 1 node, 2 CPUs
[    0.241508] smpboot: Total of 2 processors activated (9585.60 BogoMIPS)
[    0.244533] Memory: 168376K/523900K available (23836K kernel code, 4957K rwdata, 16776K rodata, 5304K init, 5840K bss, 349456K reserved, 0K cma-reserved)
[    0.245501] devtmpfs: initialized
[    0.245680] x86/mm: Memory block size: 128MB
[    0.247459] posixtimers hash table entries: 1024 (order: 2, 16384 bytes, linear)
[    0.247465] futex hash table entries: 512 (32768 bytes on 1 NUMA nodes, total 32 KiB, linear).
[    0.247800] PM: RTC time: 01:10:59, date: 2026-11-20
[    0.249349] NET: Registered PF_NETLINK/PF_ROUTE protocol family
[    0.249434] DMA: preallocated 128 KiB GFP_KERNEL|GFP_DMA pool for atomic allocations
[    0.249434] DMA: preallocated 128 KiB GFP_KERNEL|GFP_DMA32 pool for atomic allocations
[    0.249434] audit: initializing netlink subsys (disabled)
[    0.249635] thermal_sys: Registered thermal governor 'fair_share'
[    0.249637] thermal_sys: Registered thermal governor 'bang_bang'
[    0.249638] thermal_sys: Registered thermal governor 'step_wise'
[    0.249639] thermal_sys: Registered thermal governor 'user_space'
[    0.249640] thermal_sys: Registered thermal governor 'power_allocator'
[    0.249656] audit: type=2000 audit(1795137059.218:1): state=initialized audit_enabled=0 res=1
[    0.251447] cpuidle: using governor ladder
[    0.252442] cpuidle: using governor menu
[    0.254434] acpiphp: ACPI Hot Plug PCI Controller Driver version: 0.5
[    0.255526] PCI: ECAM [mem 0xe8000000-0xe80fffff] (base 0xe8000000) for domain 0000 [bus 00-00]
[    0.255533] PCI: ECAM [mem 0xe8000000-0xe80fffff] reserved as E820 entry
[    0.255548] PCI: Using configuration type 1 for base access
[    0.256508] kprobes: kprobe jump-optimization is enabled. All kprobes are optimized if possible.
[    0.257434] HugeTLB: registered 1.00 GiB page size, pre-allocated 0 pages
[    0.257434] HugeTLB: 16380 KiB vmemmap can be freed for a 1.00 GiB page
[    0.257434] HugeTLB: registered 2.00 MiB page size, pre-allocated 0 pages
[    0.257434] HugeTLB: 28 KiB vmemmap can be freed for a 2.00 MiB page
[    0.258232] ACPI: Added _OSI(Module Device)
[    0.258234] ACPI: Added _OSI(Processor Device)
[    0.258236] ACPI: Added _OSI(Processor Aggregator Device)
[    0.258434] ACPI: 1 ACPI AML tables successfully acquired and loaded
[    0.271452] ACPI: \_SB_: platform _OSC: OS support mask [006e7eff]
[    0.272149] ACPI: Interpreter enabled
[    0.272171] ACPI: PM: (supports S0 S5)
[    0.272173] ACPI: Using IOAPIC for interrupt routing
[    0.272384] PCI: Using host bridge windows from ACPI; if necessary, use "pci=nocrs" and report a bug
[    0.272386] PCI: Using E820 reservations for host bridge windows
[    0.286172] ACPI: PCI Root Bridge [PC00] (domain 0000 [bus 00])
[    0.286190] acpi PNP0A08:00: _OSC: OS supports [ExtendedConfig ASPM ClockPM Segments MSI EDR HPX-Type3]
[    0.286200] acpi PNP0A08:00: _OSC: OS requested [PCIeHotplug SHPCHotplug PME AER PCIeCapability LTR DPC]
[    0.286203] acpi PNP0A08:00: _OSC: platform willing to grant [PCIeHotplug SHPCHotplug PME AER PCIeCapability LTR DPC]
[    0.286206] acpi PNP0A08:00: _OSC: platform retains control of PCIe features (AE_ERROR)
[    0.291794] acpiphp: Slot [0] registered
[    0.291843] acpiphp: Slot [1] registered
[    0.291885] acpiphp: Slot [2] registered
[    0.291926] acpiphp: Slot [3] registered
[    0.291967] acpiphp: Slot [4] registered
[    0.292008] acpiphp: Slot [5] registered
[    0.292049] acpiphp: Slot [6] registered
[    0.292090] acpiphp: Slot [7] registered
[    0.292131] acpiphp: Slot [8] registered
[    0.292172] acpiphp: Slot [9] registered
[    0.292213] acpiphp: Slot [10] registered
[    0.292254] acpiphp: Slot [11] registered
[    0.292298] acpiphp: Slot [12] registered
[    0.292339] acpiphp: Slot [13] registered
[    0.292391] acpiphp: Slot [14] registered
[    0.293456] acpiphp: Slot [15] registered
[    0.293523] acpiphp: Slot [16] registered
[    0.293564] acpiphp: Slot [17] registered
[    0.293605] acpiphp: Slot [18] registered
[    0.293647] acpiphp: Slot [19] registered
[    0.293698] acpiphp: Slot [20] registered
[    0.293739] acpiphp: Slot [21] registered
[    0.293779] acpiphp: Slot [22] registered
[    0.293819] acpiphp: Slot [23] registered
[    0.293860] acpiphp: Slot [24] registered
[    0.293900] acpiphp: Slot [25] registered
[    0.293941] acpiphp: Slot [26] registered
[    0.293982] acpiphp: Slot [27] registered
[    0.294030] acpiphp: Slot [28] registered
[    0.294070] acpiphp: Slot [29] registered
[    0.294111] acpiphp: Slot [30] registered
[    0.294152] acpiphp: Slot [31] registered
[    0.294264] PCI host bridge to bus 0000:00
[    0.294308] pci_bus 0000:00: root bus resource [mem 0xe8000000-0xe80fffff]
[    0.294312] pci_bus 0000:00: root bus resource [mem 0xc0000000-0xe7ffffff window]
[    0.294314] pci_bus 0000:00: root bus resource [mem 0x100000000-0x3fffffeeffff window]
[    0.294317] pci_bus 0000:00: root bus resource [io  0x0000-0x0cf7 window]
[    0.294319] pci_bus 0000:00: root bus resource [io  0x0d00-0xffff window]
[    0.294321] pci_bus 0000:00: root bus resource [bus 00]
[    0.294487] pci 0000:00:00.0: [8086:0d57] type 00 class 0x060000 conventional PCI endpoint
[    0.296260] pci 0000:00:01.0: [1af4:1042] type 00 class 0x018000 conventional PCI endpoint
[    0.296943] pci 0000:00:01.0: BAR 0 [mem 0xe7f80000-0xe7ffffff]
[    0.298850] pci 0000:00:02.0: [1af4:1044] type 00 class 0xffff00 conventional PCI endpoint
[    0.299556] pci 0000:00:02.0: BAR 0 [mem 0x3fffff800000-0x3fffff87ffff 64bit]
[    0.301320] pci 0000:00:03.0: [1af4:1053] type 00 class 0xffff00 conventional PCI endpoint
[    0.302027] pci 0000:00:03.0: BAR 0 [mem 0x3fffff000000-0x3fffff07ffff 64bit]
[    0.313965] iommu: Default domain type: Translated
[    0.313967] iommu: DMA domain TLB invalidation policy: lazy mode
[    0.314698] SCSI subsystem initialized
[    0.314773] ACPI: bus type USB registered
[    0.314795] usbcore: registered new interface driver usbfs
[    0.314805] usbcore: registered new interface driver hub
[    0.314816] usbcore: registered new device driver usb
[    0.314816] pps_core: LinuxPPS API ver. 1 registered
[    0.314816] pps_core: Software ver. 5.3.6 - Copyright 2005-2007 Rodolfo Giometti <giometti@linux.it>
[    0.314816] PTP clock support registered
[    0.316457] EDAC MC: Ver: 3.0.0
[    0.319422] NetLabel: Initializing
[    0.319425] NetLabel:  domain hash size = 128
[    0.319426] NetLabel:  protocols = UNLABELED CIPSOv4 CALIPSO
[    0.319460] NetLabel:  unlabeled traffic allowed by default
[    0.319540] mctp: management component transport protocol core
[    0.319542] NET: Registered PF_MCTP protocol family
[    0.319557] PCI: Using ACPI for IRQ routing
[    0.319772] vgaarb: loaded
[    0.321402] clocksource: Switched to clocksource kvm-clock
[    0.324999] VFS: Disk quotas dquot_6.6.0
[    0.325012] VFS: Dquot-cache hash table entries: 512 (order 0, 4096 bytes)
[    0.329765] AppArmor: AppArmor Filesystem Enabled
[    0.329788] acpi PNP0C02:00: Reserved [mem 0xe8000000-0xe80fffff]
[    0.329809] pnp: PnP ACPI init
[    0.330161] pnp: PnP ACPI: found 1 devices
[    0.337067] clocksource: acpi_pm: mask: 0xffffff max_cycles: 0xffffff, max_idle_ns: 2085701024 ns
[    0.337160] NET: Registered PF_INET protocol family
[    0.337194] IP idents hash table entries: 8192 (order: 4, 65536 bytes, linear)
[    0.354901] tcp_listen_portaddr_hash hash table entries: 256 (order: 0, 4096 bytes, linear)
[    0.354916] Table-perturb hash table entries: 65536 (order: 6, 262144 bytes, linear)
[    0.354930] TCP established hash table entries: 4096 (order: 3, 32768 bytes, linear)
[    0.354940] TCP bind hash table entries: 4096 (order: 5, 131072 bytes, linear)
[    0.354947] TCP: Hash tables configured (established 4096 bind 4096)
[    0.355003] MPTCP token hash table entries: 512 (order: 2, 12288 bytes, linear)
[    0.355018] UDP hash table entries: 256 (order: 2, 16384 bytes, linear)
[    0.355023] UDP-Lite hash table entries: 256 (order: 2, 16384 bytes, linear)
[    0.355074] NET: Registered PF_UNIX/PF_LOCAL protocol family
[    0.355084] NET: Registered PF_XDP protocol family
[    0.355095] pci_bus 0000:00: resource 4 [mem 0xe8000000-0xe80fffff]
[    0.355099] pci_bus 0000:00: resource 5 [mem 0xc0000000-0xe7ffffff window]
[    0.355102] pci_bus 0000:00: resource 6 [mem 0x100000000-0x3fffffeeffff window]
[    0.355104] pci_bus 0000:00: resource 7 [io  0x0000-0x0cf7 window]
[    0.355106] pci_bus 0000:00: resource 8 [io  0x0d00-0xffff window]
[    0.355252] PCI: CLS 0 bytes, default 64
[    0.355498] clocksource: tsc: mask: 0xffffffffffffffff max_cycles: 0x228aeee0e7a, max_idle_ns: 440795314296 ns
[    0.356327] clocksource: Switched to clocksource tsc
[    0.356379] platform rtc_cmos: registered platform RTC device (no PNP device found)
[    0.357022] Initialise system trusted keyrings
[    0.357033] Key type blacklist registered
[    0.357122] workingset: timestamp_bits=36 max_order=17 bucket_order=0
[    0.357559] squashfs: version 4.0 (2009/01/31) Phillip Lougher
[    0.357693] fuse: init (API version 7.45)
[    0.364530] integrity: Platform Keyring initialized
[    0.364536] integrity: Machine keyring initialized
[    0.377340] Key type asymmetric registered
[    0.377344] Asymmetric key parser 'x509' registered
[    0.377381] Block layer SCSI generic (bsg) driver version 0.4 loaded (major 242)
[    0.378435] io scheduler mq-deadline registered
[    0.382088] ledtrig-cpu: registered to indicate activity on CPUs
[    0.382796] input: Power Button as /devices/platform/PNP0C0C:00/input/input0
[    0.382829] ACPI: button: Power : in-guest lane
