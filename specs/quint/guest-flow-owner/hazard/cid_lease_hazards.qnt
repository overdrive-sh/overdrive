// -*- mode: Bluespec; -*-
// Teeth for module C (../cid_lease.qnt): each instance is the design with ONE rule of D16-CLAIM
// switched off (OFF). The only CID holders outside the pool are VMMs that survived a serve crash
// (K-C2, A-31). Expected verdicts are in ../checks.toml.

// Check-then-claim (ADR-0170 rejected alternative): assign only checks the CID is free; the VMM claims
// it at device creation
module lease_bugCheckThenClaim { import cid_lease(OFF = Set("checkThenClaim"), CFG = "ok").* from "../cid_lease" }
// The claim is released before the VMM holds it (serve closes its reference at take_claim)
module lease_bugReleaseBeforeVmm { import cid_lease(OFF = Set("releaseBeforeVmm"), CFG = "ok").* from "../cid_lease" }
// InUse offsets are remembered per workload and skipped later without asking the kernel
module lease_bugRemember { import cid_lease(OFF = Set("rememberInUse"), CFG = "ok").* from "../cid_lease" }
// ... the liveness consequence: a workload refuses forever while a free offset's CID stays unheld
module lease_live_bugRemember { import cid_lease(OFF = Set("rememberInUse"), CFG = "live").* from "../cid_lease" }
