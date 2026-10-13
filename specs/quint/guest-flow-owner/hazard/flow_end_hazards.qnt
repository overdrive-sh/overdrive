// -*- mode: Bluespec; -*-
// Hazard instances of the flow end (../flow_end.qnt): the design with ONE rule or kernel assumption
// switched off (OFF). Expected verdicts are in ../checks.toml.

// A6 off: every TCP-source EOF is a clean half-close (a destination reset is delivered as a clean EOF)
module end_bugNoTcpEndCheck { import flow_end(OFF = Set("noTcpEndCheck"), CFG = "abort").* from "../flow_end" }
// R2-END off: a vsock end is a half-close whatever its class (a 0x3 abort is read as a half-close)
module end_bugEofIsHalfClose { import flow_end(OFF = Set("eofIsHalfClose"), CFG = "abort").* from "../flow_end" }
// R2-END off: a 0x3 aborts even when no direction is open (a clean close aborts the peer's pending drain)
module end_bugAnyShutdownAborts { import flow_end(OFF = Set("anyShutdownAborts"), CFG = "all").* from "../flow_end" }
// A4 off: an end with no classification record is ignored
// A4 off together with A5 failing: a skipped SHUTDOWN record strands the flow
module end_bugPerfTracepointNoA4 { import flow_end(OFF = Set("perfTracepoint", "noUnannouncedAbort"), CFG = "clean").* from "../flow_end" }
module end_bugNoUnannouncedAbort { import flow_end(OFF = Set("noUnannouncedAbort"), CFG = "ring").* from "../flow_end" }
// A5 off (perf-attached tracepoint): a SHUTDOWN record can be skipped silently; the end is then an unannounced end and aborts
// (a spurious abort of a clean flow)
module end_bugPerfTracepoint { import flow_end(OFF = Set("perfTracepoint"), CFG = "clean").* from "../flow_end" }
// A5 off: the EOF wake can precede the record
module end_bugWakeBeforeRecord { import flow_end(OFF = Set("wakeBeforeRecord"), CFG = "clean").* from "../flow_end" }
