// -*- mode: Bluespec; -*-
// Teeth for ../restart_authority.qnt: each instance is the design with ONE decision's rule switched off (OFF), on the
// two-attempt checking instance (CEIL = 1). Expected verdicts are in ../checks.toml.

// D5: the startup-failure guard also accepts a Failed (crashed) row
module ra_bugGuardAcceptsFailed { import restart_authority(CEIL = 1, OFF = Set("guardAcceptsFailed"), REC = false, GLFREE = false).* from "../restart_authority" }
// D6: ServiceLifecycle finalises a crash row with early_exit while WorkloadLifecycle keeps restarting it
module ra_bugSlFinalisesCrash { import restart_authority(CEIL = 1, OFF = Set("slFinalisesCrash"), REC = false, GLFREE = false).* from "../restart_authority" }
// D7: ServiceLifecycle writes startup_probe_failed on a live row (no stop) while WorkloadLifecycle keeps restarting it
module ra_bugSlFinalisesStartup { import restart_authority(CEIL = 1, OFF = Set("slFinalisesStartup"), REC = false, GLFREE = false).* from "../restart_authority" }
// D8: the single-shot dedup (terminal_announced stamped at emission): a stop that did not land is never re-emitted
module ra_bugStampAtEmission { import restart_authority(CEIL = 1, OFF = Set("stampAtEmission"), REC = false, GLFREE = false).* from "../restart_authority" }
// B2: the hook runs before the terminal row write, so a stop's row-write failure has already removed the supervisor
module ra_bugHookBeforeRowWrite { import restart_authority(CEIL = 1, OFF = Set("hookBeforeRowWrite"), REC = false, GLFREE = false).* from "../restart_authority" }
