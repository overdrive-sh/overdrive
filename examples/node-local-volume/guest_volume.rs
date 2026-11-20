//! Guest program for the node-local volume example.
//!
//! On every start it reads the notes an earlier start wrote to the guest's
//! second disk (virtio serial `ovd-volume`), appends one note, and serves the
//! outcome over HTTP. A note is one 512-byte sector of text, so the host can
//! read the same notes straight from the volume's device node.
//!
//! `GET /ready` answers 204 only when every earlier note read back intact and
//! the new note was flushed; otherwise 503. `GET /records` answers the
//! read-back, one line per note, so an operator sees what survived.

use std::fs::{self, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::net::TcpListener;
use std::path::PathBuf;

const SECTOR: usize = 512;
const MAGIC: &str = "OVD-VOLUME-NOTE ";
const MAX_NOTES: u64 = 4096;
const VOLUME_SERIAL: &str = "ovd-volume";

struct Outcome {
    ready: bool,
    report: String,
}

fn find_device() -> Result<PathBuf, String> {
    let by_id = PathBuf::from("/dev/disk/by-id/virtio-ovd-volume");
    if by_id.exists() {
        return Ok(by_id);
    }
    let blocks = fs::read_dir("/sys/block").map_err(|e| format!("cannot list /sys/block: {e}"))?;
    for entry in blocks.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let serial = fs::read_to_string(entry.path().join("serial")).unwrap_or_default();
        if name.starts_with("vd") && serial.trim() == VOLUME_SERIAL {
            return Ok(PathBuf::from(format!("/dev/{name}")));
        }
    }
    Err(format!("no disk with virtio serial {VOLUME_SERIAL} is attached"))
}

/// Reads the notes a previous start wrote: sector `i` must say `seq=<i+1>`.
fn read_notes(device: &PathBuf) -> Result<Vec<String>, String> {
    let mut disk = OpenOptions::new()
        .read(true)
        .open(device)
        .map_err(|e| format!("cannot open {}: {e}", device.display()))?;
    let mut notes = Vec::new();
    for index in 0..MAX_NOTES {
        let mut sector = [0_u8; SECTOR];
        if disk.read_exact(&mut sector).is_err() {
            break;
        }
        let text = String::from_utf8_lossy(&sector);
        let line = text.trim_end_matches('\0').trim_end();
        if !line.starts_with(MAGIC) {
            break;
        }
        if !line[MAGIC.len()..].starts_with(&format!("seq={} ", index + 1)) {
            return Err(format!("note {} is out of sequence: {line}", index + 1));
        }
        notes.push(line.to_owned());
    }
    Ok(notes)
}

fn append_note(device: &PathBuf, seq: u64, prior: usize) -> Result<String, String> {
    let uptime = fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|text| text.split_whitespace().next().map(str::to_owned))
        .unwrap_or_else(|| "unknown".to_owned());
    let line = format!("{MAGIC}seq={seq} prior_verified={prior} uptime_s={uptime}\n");
    let mut sector = [0_u8; SECTOR];
    sector[..line.len()].copy_from_slice(line.as_bytes());
    let mut disk = OpenOptions::new()
        .write(true)
        .open(device)
        .map_err(|e| format!("cannot open {} for writing: {e}", device.display()))?;
    disk.seek(SeekFrom::Start((seq - 1) * SECTOR as u64)).map_err(|e| format!("seek: {e}"))?;
    disk.write_all(&sector).map_err(|e| format!("write: {e}"))?;
    disk.sync_all().map_err(|e| format!("flush: {e}"))?;
    Ok(line.trim_end().to_owned())
}

fn run(device: Result<PathBuf, String>) -> Outcome {
    let device = match device {
        Ok(device) => device,
        Err(reason) => {
            return Outcome { ready: false, report: format!("volume: unavailable ({reason})\n") };
        }
    };
    let mut report = format!("volume: {}\n", device.display());
    let prior = match read_notes(&device) {
        Ok(notes) => notes,
        Err(reason) => {
            report.push_str(&format!("read-back: FAILED ({reason})\n"));
            return Outcome { ready: false, report };
        }
    };
    report.push_str(&format!("read-back: {} earlier note(s) intact\n", prior.len()));
    for note in &prior {
        report.push_str(&format!("earlier: {note}\n"));
    }
    if prior.len() as u64 >= MAX_NOTES {
        report.push_str("write: FAILED (the note area is full)\n");
        return Outcome { ready: false, report };
    }
    match append_note(&device, prior.len() as u64 + 1, prior.len()) {
        Ok(note) => {
            report.push_str(&format!("written: {note}\n"));
            Outcome { ready: true, report }
        }
        Err(reason) => {
            report.push_str(&format!("write: FAILED ({reason})\n"));
            Outcome { ready: false, report }
        }
    }
}

fn respond(status: &str, body: &str) -> String {
    format!(
        "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    )
}

fn main() {
    let mut port = 18_080_u16;
    let mut device_override = None;
    let mut args = std::env::args().skip(1);
    while let Some(flag) = args.next() {
        let value = args.next().unwrap_or_else(|| panic!("missing value for {flag}"));
        match flag.as_str() {
            "--port" => port = value.parse().unwrap_or_else(|_| panic!("invalid port {value}")),
            "--device" => device_override = Some(PathBuf::from(value)),
            _ => panic!("unknown argument {flag}"),
        }
    }
    let outcome = run(device_override.map_or_else(find_device, Ok));
    print!("{}", outcome.report);
    let listener = TcpListener::bind(("0.0.0.0", port)).expect("bind guest listener");
    for incoming in listener.incoming() {
        let mut stream = incoming.expect("accept connection");
        let mut request = [0_u8; 1024];
        let count = stream.read(&mut request).unwrap_or(0);
        let request = String::from_utf8_lossy(&request[..count]);
        let response = if request.starts_with("GET /ready ") {
            if outcome.ready {
                respond("204 No Content", "")
            } else {
                respond("503 Service Unavailable", "")
            }
        } else if request.starts_with("GET /records ") {
            respond("200 OK", &outcome.report)
        } else {
            respond("404 Not Found", "")
        };
        let _ = stream.write_all(response.as_bytes());
    }
}
