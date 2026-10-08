// -*- mode: Bluespec; -*-
// Teeth for steering concern 1 (../prefix_boot.qnt): each instance is the design with ONE rule
// switched off (OFF). Expected verdicts are in ../checks.toml.

// D8a-ROUTE: the route step runs before the steering is loaded and swapped in
module boot_bugRouteFirst { import prefix_boot(OFF = Set("routeFirst"), CFG = "ok").* from "../prefix_boot" }
// D8a-ROUTE / D8a-LOAD-SWAP: a load or swap failure is ignored and boot continues to the route
module boot_bugIgnoreFailure { import prefix_boot(OFF = Set("ignoreFailure"), CFG = "ok").* from "../prefix_boot" }
// D8a-LOOKUP / D8a-LOAD-SWAP: swap detaches the pinned link and attaches anew (non-atomic swap)
module boot_bugDetach { import prefix_boot(OFF = Set("detach"), CFG = "ok").* from "../prefix_boot" }
// D8a-LOOKUP / ADR-0159: the steering is process-owned, never pinned
module boot_bugUnpinned { import prefix_boot(OFF = Set("unpinned"), CFG = "ok").* from "../prefix_boot" }
