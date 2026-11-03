//! The only `unsafe` module of the host gateway relay.
//!
//! SCAFFOLD: true. It owns, and only it may contain, the raw calls the relay
//! needs: `setsockopt` for `TCP_ULP`, `TLS_TX` and `TLS_RX` (arm and re-arm),
//! `recvmsg` with a control buffer for `TLS_GET_RECORD_TYPE`, `splice`, `pipe2`
//! and `fcntl(F_SETPIPE_SZ / F_GETPIPE_SZ)`. The DESIGN pins this ownership
//! boundary and no function signature in it; the crafter shapes the safe
//! wrappers, so none is scaffolded here.
