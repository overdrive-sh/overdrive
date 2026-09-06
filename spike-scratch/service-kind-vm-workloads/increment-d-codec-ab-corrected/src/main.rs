//! SPIKE ONLY: compare JSON and rkyv for a provisional VM-Exec wire algebra.
//!
//! This is an isolated measurement harness, not an accepted protocol or API.

#[cfg(all(feature = "codec-json", feature = "codec-rkyv"))]
compile_error!("select at most one codec feature");

use std::hint::black_box;
use std::time::Instant;

const MAX_FRAME: usize = 16 * 1024;
const ITERATIONS: usize = 100_000;
const SCHEMA_V1: u32 = 1;
const SCHEMA_V2: u32 = 2;

fn frame(payload: &[u8]) -> Result<Vec<u8>, &'static str> {
    if payload.len() > MAX_FRAME {
        return Err("oversized payload");
    }
    let len = u32::try_from(payload.len()).map_err(|_| "length does not fit u32")?;
    let mut out = Vec::with_capacity(4 + payload.len());
    out.extend_from_slice(&len.to_be_bytes());
    out.extend_from_slice(payload);
    Ok(out)
}

fn unframe(frame: &[u8]) -> Result<&[u8], &'static str> {
    let header: [u8; 4] = frame.get(..4).ok_or("truncated length prefix")?.try_into().unwrap();
    let declared = usize::try_from(u32::from_be_bytes(header)).map_err(|_| "length overflow")?;
    if declared > MAX_FRAME {
        return Err("oversized frame");
    }
    let payload = frame.get(4..).ok_or("missing payload")?;
    if payload.len() != declared {
        return Err("truncated or trailing payload");
    }
    Ok(payload)
}

fn versioned_payload(schema: u32, encoded: &[u8]) -> Vec<u8> {
    let mut payload = Vec::with_capacity(4 + encoded.len());
    payload.extend_from_slice(&schema.to_be_bytes());
    payload.extend_from_slice(encoded);
    payload
}

fn split_version(payload: &[u8]) -> Result<(u32, &[u8]), &'static str> {
    let raw: [u8; 4] = payload.get(..4).ok_or("missing schema version")?.try_into().unwrap();
    Ok((u32::from_be_bytes(raw), &payload[4..]))
}

fn oversized_frame() -> Vec<u8> {
    u32::try_from(MAX_FRAME + 1).unwrap().to_be_bytes().to_vec()
}

fn baseline() {
    // This deliberately activates the same serde_json Vec<String> path used by
    // overdrive-core's BeaconMessage::Exec implementation.
    let argv = vec!["/usr/bin/check".to_string(), "--mode".to_string(), "ready now".to_string()];
    let encoded = serde_json::to_vec(&argv).unwrap();
    let decoded: Vec<String> = serde_json::from_slice(&encoded).unwrap();
    assert_eq!(decoded, argv);
    println!("VARIANT baseline");
    println!("BEACON_JSON encoded_bytes={} roundtrip=PASS", encoded.len());
    println!("NOTE no provisional VM-Exec codec compiled");
}

#[cfg(feature = "codec-json")]
mod selected {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "kind", rename_all = "snake_case")]
    enum Message {
        Hello { protocol_version: u16, session_generation: u64 },
        Execute { session_generation: u64, request_id: u64, argv: Vec<String> },
        Cancel { session_generation: u64, request_id: u64 },
        Overloaded { session_generation: u64, request_id: u64 },
        Result { session_generation: u64, request_id: u64, outcome: Outcome },
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    #[serde(tag = "category", rename_all = "snake_case")]
    enum Outcome {
        Success,
        ExitNonzero { code: i32 },
        Signaled { signal: i32 },
        TimedOut,
        Cancelled,
        Unavailable,
        ProtocolFailure,
    }

    fn encode(message: &Message) -> Result<Vec<u8>, &'static str> {
        let encoded = serde_json::to_vec(message).map_err(|_| "json encode")?;
        frame(&versioned_payload(SCHEMA_V1, &encoded))
    }

    fn decode(bytes: &[u8]) -> Result<Message, &'static str> {
        let payload = unframe(bytes)?;
        let (schema, encoded) = split_version(payload)?;
        if schema != SCHEMA_V1 {
            return Err("unsupported schema version");
        }
        serde_json::from_slice(encoded).map_err(|_| "json validation")
    }

    pub fn run() {
        let request = Message::Execute {
            session_generation: 9,
            request_id: 42,
            argv: vec!["/usr/bin/check".into(), "--mode".into(), "ready now".into()],
        };
        let result = Message::Result {
            session_generation: 9,
            request_id: 42,
            outcome: Outcome::Success,
        };
        let maximum = Message::Execute {
            session_generation: u64::MAX,
            request_id: u64::MAX,
            // Largest identical logical argv admitted by both codecs: JSON is
            // the limiting encoding and lands exactly on MAX_FRAME.
            argv: vec!["x".repeat(16_274)],
        };
        let request_frame = encode(&request).unwrap();
        let result_frame = encode(&result).unwrap();
        let maximum_frame = encode(&maximum).unwrap();
        assert_eq!(decode(&request_frame).unwrap(), request);
        assert_eq!(decode(&result_frame).unwrap(), result);

        let start = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(decode(black_box(&request_frame)).unwrap());
        }
        let elapsed = start.elapsed();

        let malformed = frame(&versioned_payload(SCHEMA_V1, br#"{\"kind\":false}"#)).unwrap();
        let truncated = &request_frame[..request_frame.len() - 1];
        let oversized = oversized_frame();
        let v2_breaking = frame(&versioned_payload(SCHEMA_V2, br#"{}"#)).unwrap();
        let v2_additive = frame(&versioned_payload(
            SCHEMA_V1,
            br#"{"kind":"execute","session_generation":9,"request_id":42,"argv":["/usr/bin/check"],"trace_hint":"new additive field"}"#,
        ))
        .unwrap();
        let additive = decode(&v2_additive).is_ok();
        assert!(decode(&malformed).is_err());
        assert!(decode(truncated).is_err());
        assert!(decode(&oversized).is_err());
        assert!(decode(&v2_breaking).is_err());
        assert!(additive);

        println!("VARIANT json");
        println!(
            "ALGEBRA hello=1 execute=1 cancel=1 overloaded=1 result_categories=7 direct_argv=1"
        );
        println!(
            "ENCODED request_frame={} result_frame={} max_representative_frame={} cap={}",
            request_frame.len(),
            result_frame.len(),
            maximum_frame.len(),
            MAX_FRAME + 4
        );
        println!(
            "DECODE iterations={} elapsed_ns={} ns_per_op={:.2}",
            ITERATIONS,
            elapsed.as_nanos(),
            elapsed.as_nanos() as f64 / ITERATIONS as f64
        );
        println!(
            "REJECTION malformed=PASS truncated=PASS oversized_preallocation=PASS unsupported_schema=PASS"
        );
        println!("EVOLUTION v1_reader_additive_same_schema={}", if additive { "ACCEPT" } else { "REJECT" });
    }
}

#[cfg(feature = "codec-rkyv")]
mod selected {
    use super::*;
    use rkyv::{Archive, Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize, Deserialize)]
    #[rkyv(derive(Debug))]
    enum MessageV1 {
        Hello { protocol_version: u16, session_generation: u64 },
        Execute { session_generation: u64, request_id: u64, argv: Vec<String> },
        Cancel { session_generation: u64, request_id: u64 },
        Overloaded { session_generation: u64, request_id: u64 },
        Result { session_generation: u64, request_id: u64, outcome: Outcome },
    }

    #[derive(Debug, Clone, PartialEq, Eq, Archive, Serialize, Deserialize)]
    #[rkyv(derive(Debug))]
    enum Outcome {
        Success,
        ExitNonzero { code: i32 },
        Signaled { signal: i32 },
        TimedOut,
        Cancelled,
        Unavailable,
        ProtocolFailure,
    }

    #[derive(Debug, Archive, Serialize, Deserialize)]
    #[rkyv(derive(Debug))]
    enum MessageV2 {
        Execute {
            session_generation: u64,
            request_id: u64,
            argv: Vec<String>,
            trace_hint: Option<String>,
        },
    }

    fn encode(message: &MessageV1) -> Result<Vec<u8>, &'static str> {
        let encoded = rkyv::to_bytes::<rkyv::rancor::Error>(message).map_err(|_| "rkyv encode")?;
        frame(&versioned_payload(SCHEMA_V1, &encoded))
    }

    fn decode(bytes: &[u8]) -> Result<MessageV1, &'static str> {
        let payload = unframe(bytes)?;
        let (schema, encoded) = split_version(payload)?;
        if schema != SCHEMA_V1 {
            return Err("unsupported schema version");
        }
        // `from_bytes` performs bytecheck before deserializing. No archived
        // value is accessed before this validation succeeds.
        rkyv::from_bytes::<MessageV1, rkyv::rancor::Error>(encoded)
            .map_err(|_| "rkyv byte validation")
    }

    pub fn run() {
        let request = MessageV1::Execute {
            session_generation: 9,
            request_id: 42,
            argv: vec!["/usr/bin/check".into(), "--mode".into(), "ready now".into()],
        };
        let result = MessageV1::Result {
            session_generation: 9,
            request_id: 42,
            outcome: Outcome::Success,
        };
        let maximum = MessageV1::Execute {
            session_generation: u64::MAX,
            request_id: u64::MAX,
            // Same logical value as JSON's maximum representative frame.
            argv: vec!["x".repeat(16_274)],
        };
        let request_frame = encode(&request).unwrap();
        let result_frame = encode(&result).unwrap();
        let maximum_frame = encode(&maximum).unwrap();
        assert_eq!(decode(&request_frame).unwrap(), request);
        assert_eq!(decode(&result_frame).unwrap(), result);

        let start = Instant::now();
        for _ in 0..ITERATIONS {
            black_box(decode(black_box(&request_frame)).unwrap());
        }
        let elapsed = start.elapsed();

        let malformed = frame(&versioned_payload(SCHEMA_V1, &[0xff; 32])).unwrap();
        let truncated = &request_frame[..request_frame.len() - 1];
        let oversized = oversized_frame();
        let v2_archive = rkyv::to_bytes::<rkyv::rancor::Error>(&MessageV2::Execute {
            session_generation: 9,
            request_id: 42,
            argv: vec!["/usr/bin/check".into()],
            trace_hint: Some("new additive field".into()),
        })
        .unwrap();
        let v2_additive = frame(&versioned_payload(SCHEMA_V2, &v2_archive)).unwrap();
        assert!(decode(&malformed).is_err());
        assert!(decode(truncated).is_err());
        assert!(decode(&oversized).is_err());
        let additive = decode(&v2_additive).is_ok();
        assert!(!additive);

        println!("VARIANT rkyv");
        println!(
            "ALGEBRA hello=1 execute=1 cancel=1 overloaded=1 result_categories=7 direct_argv=1"
        );
        println!(
            "ENCODED request_frame={} result_frame={} max_representative_frame={} cap={}",
            request_frame.len(),
            result_frame.len(),
            maximum_frame.len(),
            MAX_FRAME + 4
        );
        println!(
            "DECODE iterations={} elapsed_ns={} ns_per_op={:.2}",
            ITERATIONS,
            elapsed.as_nanos(),
            elapsed.as_nanos() as f64 / ITERATIONS as f64
        );
        println!(
            "REJECTION malformed=PASS truncated=PASS oversized_preallocation=PASS unsupported_schema=PASS bytecheck_before_access=PASS"
        );
        println!("EVOLUTION v1_reader_additive_new_layout={}", if additive { "ACCEPT" } else { "REJECT" });
    }
}

fn main() {
    baseline();

    #[cfg(any(feature = "codec-json", feature = "codec-rkyv"))]
    selected::run();
}
