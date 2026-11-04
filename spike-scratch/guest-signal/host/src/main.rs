//! guest-signal spike host: the production `VsockBeaconIo` (accept / read_line / write_line) listening on the beacon
//! port, with the production `BeaconMessage` codec classifying every line, plus the spike's own `LISTEN` parser.
//!
//! gshost <port> <scenario> <out.jsonl>
//!   scenario = wl            : EXEC ["wl", tcp, udp] per connection; records LISTEN/EXIT order + timing per cid
//!            | lat:<sessions>:<k>  : per connection EXEC ["lat",k,base,exit_after]; ACK every LISTEN; verify order
//!            | sat:<n>       : EXEC ["sat", n]; verify two interleaved streams, no ACK
//! After the last scenario connection the host sends EXEC ["quit"] on the next connection.
#[path = "../../common.rs"]
mod common;

use overdrive_core::guest_transport::VsockListenPort;
use overdrive_core::vm::beacon::{BeaconIo, BeaconMessage, BeaconParseError};
use overdrive_core::vm::config::VsockPort;
use overdrive_host::VsockBeaconIo;
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Instant;

static T0: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
fn now_ns() -> u64 {
    T0.get().unwrap().elapsed().as_nanos() as u64
}

fn parse_listen(l: &str) -> Option<(String, u16)> {
    let mut t = l.split(' ');
    if t.next()? != "LISTEN" {
        return None;
    }
    let p = t.next()?;
    if p != "tcp" && p != "udp" {
        return None;
    }
    let port: u16 = t.next()?.parse().ok()?;
    if port == 0 || t.next().is_some() {
        return None;
    }
    Some((p.to_string(), port))
}

#[derive(Default)]
struct Tally {
    lines: u64,
    ready: u64,
    listen_ok: u64,
    listen_out_of_order: u64,
    listen_after_exit: u64,
    exits: Vec<(u64, i32)>, // (index in LISTEN stream when it arrived, status)
    exit_out_of_order: u64,
    malformed: Vec<String>,
    today_codec_unknown_kind: u64, // production BeaconMessage::from_str rejected a (valid) LISTEN line
    today_codec_other_err: u64,
    stats: Vec<String>,
    rtts: Vec<u64>,
}

async fn handle(io: Arc<VsockBeaconIo>, sock: overdrive_core::vm::beacon::BeaconSocket, key: overdrive_core::vm::beacon::BeaconConnKey, scenario: String, seq: u64, out: Arc<Mutex<std::fs::File>>, sessions_done: Arc<AtomicU64>, total: u64) {
    let cid = key.peer_cid.get();
        let mut tally = Tally::default();
    let t_conn = now_ns();
    // READY
    let ready = io.read_line(&sock).await;
    let t_ready = now_ns();
    match &ready {
        Ok(Some(l)) => match l.parse::<BeaconMessage>() {
            Ok(BeaconMessage::Ready { .. }) => tally.ready += 1,
            other => tally.malformed.push(format!("first line not READY: {l:?} -> {other:?}")),
        },
        other => tally.malformed.push(format!("no READY: {other:?}")),
    }
    let sc: Vec<&str> = scenario.split(':').collect();
    let mut base = 0u64;
    let exec_line = if seq >= total && sc[0] != "wl" {
        "EXEC [\"quit\"]".to_string()
    } else {
        match sc[0] {
            "wl" => {
                let tcp = 10_000 + (cid % 20_000);
                let udp = 30_000 + (cid % 20_000);
                format!("EXEC [\"wl\",\"/gswork\",\"{tcp}\",\"{udp}\"]")
            }
            "lat" => {
                let k: u64 = sc[2].parse().unwrap();
                base = seq * k;
                // pseudo-random exit point, deterministic per session
                let exit_after = (seq.wrapping_mul(2654435761) % k).max(1);
                format!("EXEC [\"lat\",\"{k}\",\"{base}\",\"{exit_after}\"]")
            }
            "sat" => format!("EXEC [\"sat\",\"{}\",\"{}\"]", sc[1], sc.get(2).copied().unwrap_or("free")),
            _ => unreachable!(),
        }
    };
    let t_exec0 = now_ns();
    io.write_line(&sock, &exec_line).await.expect("write EXEC");
    let t_exec = now_ns();
    if exec_line.contains("quit") {
        io.close(sock).await;
        return;
    }
    let (want_tcp, want_udp) = (10_000 + (cid % 20_000), 30_000 + (cid % 20_000));
    let mut listen_idx = base; // next expected LISTEN idx (lat / sat)
    let mut exit_seen_at: Option<u64> = None;
    let mut sat_exit_next = 0i32;
    let mut wl_listens: Vec<(String, u16, u64, bool)> = vec![]; // (proto, port, t_ns, before_exec_write)
    let mut wl_exit: Option<(i32, u64)> = None;
    let mut last_read_ns;
    loop {
        let line = match tokio::time::timeout(std::time::Duration::from_secs(if sc[0] == "wl" { 5 } else { 120 }), io.read_line(&sock)).await {
            Ok(Ok(Some(l))) => l,
            Ok(Ok(None)) => break,
            Ok(Err(e)) => {
                tally.malformed.push(format!("read err {e}"));
                break;
            }
            Err(_) => {
                tally.malformed.push("read timeout".into());
                break;
            }
        };
        last_read_ns = now_ns();
        tally.lines += 1;
        // production codec verdict
        let prod = line.parse::<BeaconMessage>();
        if let Some((p, port)) = parse_listen(&line) {
            match &prod {
                Err(BeaconParseError::UnknownKind { .. }) => tally.today_codec_unknown_kind += 1,
                Err(_) => tally.today_codec_other_err += 1,
                Ok(_) => {}
            }
            match sc[0] {
                "wl" => wl_listens.push((p, port, last_read_ns, last_read_ns < t_exec0)),
                _ => {
                    let (ep, eport) = common::expected(listen_idx);
                    if p == ep && port == eport {
                        tally.listen_ok += 1;
                    } else {
                        tally.listen_out_of_order += 1;
                    }
                    if exit_seen_at.is_some() {
                        tally.listen_after_exit += 1;
                    }
                    if sc[0] == "lat" {
                        let _ = io.write_line(&sock, &format!("ACK {listen_idx}")).await;
                    }
                    listen_idx += 1;
                }
            }
            if sc[0] == "wl" && exit_seen_at.is_some() {
                tally.listen_after_exit += 1;
            }
            continue;
        }
        match prod {
            Ok(BeaconMessage::Exit { status }) => {
                if sc[0] == "sat" {
                    if status != sat_exit_next {
                        tally.exit_out_of_order += 1;
                    }
                    sat_exit_next = status + 1;
                    tally.exits.push((listen_idx - base, status));
                } else {
                    tally.exits.push((listen_idx - base, status));
                    if exit_seen_at.is_none() {
                        exit_seen_at = Some(last_read_ns);
                    }
                    if sc[0] == "wl" {
                        wl_exit = Some((status, last_read_ns));
                    }
                }
            }
            Ok(other) => tally.malformed.push(format!("unexpected message {other:?}")),
            Err(_) if line.starts_with("RTTS ") => {
                let mut it = line.splitn(4, ' ');
                it.next();
                it.next();
                let bad: u64 = it.next().unwrap_or("0").parse().unwrap_or(0);
                if bad > 0 {
                    tally.malformed.push(format!("guest saw {bad} bad ACKs"));
                }
                tally.rtts.extend(it.next().unwrap_or("").split(',').filter_map(|x| x.parse::<u64>().ok()));
            }
            Err(_) if line.starts_with("STATS ") || line.starts_with("NOTE ") => tally.stats.push(line.clone()),
            Err(e) => tally.malformed.push(format!("{line:?}: {e}")),
        }
    }
    let rec = if sc[0] == "wl" {
        let tcp_ok = wl_listens.iter().any(|(p, port, _, _)| p == "tcp" && *port == want_tcp as u16);
        let udp_ok = wl_listens.iter().any(|(p, port, _, _)| p == "udp" && *port == want_udp as u16);
        let any_before_exec = wl_listens.iter().any(|x| x.3);
        let before_exit = |t: u64| wl_exit.map(|e| t < e.1).unwrap_or(true);
        let listens_before_exit = wl_listens.iter().filter(|x| before_exit(x.2)).count();
        serde_json::json!({
            "scenario":"wl","cid":cid,"tcp_reported":tcp_ok,"udp_reported":udp_ok,
            "listens":wl_listens.len(),"listens_before_exit":listens_before_exit,
            "any_listen_before_exec_write":any_before_exec,
            "t_exec_write_to_first_listen_us": wl_listens.first().map(|x| (x.2.saturating_sub(t_exec))/1000),
            "t_ready_us_from_conn": (t_ready-t_conn)/1000,
            "exit": wl_exit.map(|e| e.0), "notes": tally.stats, "listen_after_exit": tally.listen_after_exit,
            "malformed": tally.malformed, "today_codec_unknown_kind": tally.today_codec_unknown_kind,
            "today_codec_other_err": tally.today_codec_other_err,
        })
    } else {
        let mut r = tally.rtts.clone();
        r.sort_unstable();
        let q = |p: f64| if r.is_empty() { 0 } else { r[((r.len() as f64 - 1.0) * p) as usize] };
        serde_json::json!({
            "scenario":sc[0],"cid":cid,"seq":seq,"lines":tally.lines,"ready":tally.ready,
            "listen_ok":tally.listen_ok,"listen_out_of_order":tally.listen_out_of_order,
            "listen_after_exit":tally.listen_after_exit,"exits":tally.exits.len(),
            "exit_positions": if sc[0]=="lat" {serde_json::json!(tally.exits)} else {serde_json::json!(tally.exits.len())},
            "exit_out_of_order":tally.exit_out_of_order,
            "malformed":tally.malformed,"stats":tally.stats,
            "today_codec_unknown_kind":tally.today_codec_unknown_kind,"today_codec_other_err":tally.today_codec_other_err,
            "rtt_n":r.len(),"rtt_ns":{"min":q(0.0),"p50":q(0.5),"p90":q(0.9),"p99":q(0.99),"p999":q(0.999),"max":q(1.0)},
            "rtts": tally.rtts,
        })
    };
    writeln!(out.lock().unwrap(), "{}", rec).unwrap();
    out.lock().unwrap().flush().unwrap();
    sessions_done.fetch_add(1, Ordering::SeqCst);
    io.close(sock).await;
}

#[tokio::main]
async fn main() {
    T0.set(Instant::now()).unwrap();
    let a: Vec<String> = std::env::args().collect();
    let port: u32 = a[1].parse().unwrap();
    let scenario = a[2].clone();
    let out = Arc::new(Mutex::new(std::fs::OpenOptions::new().create(true).append(true).open(&a[3]).unwrap()));
    let total: u64 = match scenario.split(':').next().unwrap() {
        "lat" => scenario.split(':').nth(1).unwrap().parse().unwrap(),
        "sat" => 1,
        _ => u64::MAX,
    };
    let io = Arc::new(VsockBeaconIo::new());
    io.bind(VsockListenPort::Fixed(VsockPort::new(port))).await.expect("bind beacon");
    println!("GSHOST_LISTENING port={port} scenario={scenario}");
    let done = Arc::new(AtomicU64::new(0));
    let mut seq = 0u64;
    loop {
        let (sock, key) = io.accept().await.expect("accept");
        let (io2, sc, out2, d2) = (io.clone(), scenario.clone(), out.clone(), done.clone());
        let s = seq;
        seq += 1;
        // lat/sat run sessions strictly one at a time (a single guest drives them); wl is concurrent
        let h = tokio::spawn(handle(io2, sock, key, sc, s, out2, d2, total));
        if !scenario.starts_with("wl") {
            h.await.unwrap();
            if s >= total {
                println!("GSHOST_DONE");
                return;
            }
        }
    }
}
