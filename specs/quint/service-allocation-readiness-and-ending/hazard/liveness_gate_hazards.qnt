// -*- mode: Bluespec; -*-
// Teeth for ../liveness_gate.qnt. Expected verdicts are in ../checks.toml.

// D9: the pre-D9 shape: on_alloc_running spawns the liveness tasks, so the streak accumulates during startup
module lg_bugSpawnAtRunning { import liveness_gate(OFF = Set("spawnAtRunning"), REC = false).* from "../liveness_gate" }

// B1: Stable emission is guarded by a record that a Stable write was attempted, set at emission: a Stable whose write failed is never emitted again
module lg_bugStampAtEmission { import liveness_gate(OFF = Set("stampAtEmission"), REC = false).* from "../liveness_gate" }
