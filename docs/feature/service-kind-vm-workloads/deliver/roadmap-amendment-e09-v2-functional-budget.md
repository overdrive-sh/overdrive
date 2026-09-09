# Roadmap amendment — E09-v2 functional acceptance budget

Date: 2026-09-08  
Status: approved for bounded roadmap execution; native verification remains pending

This focused amendment changes only the current E09-v2 acceptance sample and
its bounded remote lifetime. E09-v2 now means exactly 20 uniquely identified
healthy/failure pairs, scheduled as two ten-pair cohorts at concurrency 10,
through one build, one reusable-artifact preparation, and one unchanged
control-plane process. Each pair retains the existing healthy and
`StartupProbeFailed` public oracles, cleanup complements, cohort barriers,
and ordered partial ledger. No pair is retried, replaced, or discarded.

The remote example owner receives one 600-second setup-and-trials budget,
including build, preparation, serve startup, and both cohorts, followed by a
separate 60-second bounded cleanup grace. The parent transport wait is longer
than those remote windows and remains independently bounded. A timeout is a
failure (exit 124 at the remote timeout boundary where applicable); partial
ledger, timing, identity, and transcript output is retained before ordinary
materialization cleanup. The amendment does not change shared SSH/lease
infrastructure, reclamation ownership, guest negative-observation contracts,
or the existing cohort scheduler into a rolling pool.

The 20-pair sample is bounded functional acceptance, not reliability,
capacity, or throughput proof. Seeded `overdrive-sim` ordering evidence stays
independent. A longer soak is optional future documentation and is not a
mandatory mode or gate. Native verification remains pending; no native run,
DELIVER step 02-04 execution, GREEN result, or completion is claimed here.

Active surfaces are aligned to
`verification/expectations/E09-v2-vm-service-tcp-truthfulness-20/`, selector
`tcp-truthfulness-20`, S-SVM-25, the v2 checked-in example, and the active
expectations index. The prior ADR-0101/E09-v2 100-pair amendment and its
independent review remain historical provenance; E09-v1, historical 100-pair
captures, and failed DES history are not relabeled or overwritten.

Independent validation is required before this amendment is executable as a
roadmap approval. `deliver/roadmap.json` records that pending status and keeps
the prior approval identifiers and scopes as provenance. The acceptance
designer's bounded RED/GREEN handoff is recorded separately in
`docs/feature/service-kind-vm-workloads/distill/e09-v2-functional-budget-handoff.md`.
