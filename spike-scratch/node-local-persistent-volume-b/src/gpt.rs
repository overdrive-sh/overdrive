//! Minimal GPT writer (throwaway): protective MBR, primary + backup header and entry array.
use std::fs::OpenOptions;
use std::io::{Seek, SeekFrom, Write};

fn crc32(data: &[u8]) -> u32 {
    let mut c = 0xFFFF_FFFFu32;
    for &b in data { c ^= b as u32; for _ in 0..8 { c = if c & 1 != 0 { (c >> 1) ^ 0xEDB8_8320 } else { c >> 1 }; } }
    !c
}

pub struct Part { pub first_lba: u64, pub last_lba: u64, pub name: &'static str, pub type_guid: [u8; 16], pub guid: [u8; 16] }

/// Mixed-endian GUID bytes from the canonical text form.
pub fn guid(s: &str) -> [u8; 16] {
    let h: Vec<u8> = s.replace('-', "").as_bytes().chunks(2).map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap()).collect();
    let mut g = [0u8; 16];
    g[0..4].copy_from_slice(&[h[3], h[2], h[1], h[0]]);
    g[4..6].copy_from_slice(&[h[5], h[4]]);
    g[6..8].copy_from_slice(&[h[7], h[6]]);
    g[8..16].copy_from_slice(&h[8..16]);
    g
}
pub const LINUX_FS: &str = "0FC63DAF-8483-4772-8E79-3D69D8477DE4";
pub const LINUX_LVM: &str = "E6D6D379-F507-44C2-A23C-238F2A3DF928";

pub fn write_gpt(path: &str, size_bytes: u64, parts: &[Part]) {
    let mut f = OpenOptions::new().create(true).write(true).truncate(true).open(path).unwrap();
    f.set_len(size_bytes).unwrap();
    let total = size_bytes / 512;
    let last = total - 1;
    let mut entries = vec![0u8; 128 * 128];
    for (i, p) in parts.iter().enumerate() {
        let e = &mut entries[i * 128..(i + 1) * 128];
        e[0..16].copy_from_slice(&p.type_guid);
        e[16..32].copy_from_slice(&p.guid);
        e[32..40].copy_from_slice(&p.first_lba.to_le_bytes());
        e[40..48].copy_from_slice(&p.last_lba.to_le_bytes());
        for (j, u) in p.name.encode_utf16().enumerate() { e[56 + j * 2..58 + j * 2].copy_from_slice(&u.to_le_bytes()); }
    }
    let ecrc = crc32(&entries);
    let disk_guid = guid("DEADBEEF-0000-4000-8000-00000000A1B2");
    let hdr = |cur: u64, bak: u64, ents: u64| -> Vec<u8> {
        let mut h = vec![0u8; 92];
        h[0..8].copy_from_slice(b"EFI PART");
        h[8..12].copy_from_slice(&0x0001_0000u32.to_le_bytes());
        h[12..16].copy_from_slice(&92u32.to_le_bytes());
        h[24..32].copy_from_slice(&cur.to_le_bytes());
        h[32..40].copy_from_slice(&bak.to_le_bytes());
        h[40..48].copy_from_slice(&34u64.to_le_bytes());
        h[48..56].copy_from_slice(&(last - 33).to_le_bytes());
        h[56..72].copy_from_slice(&disk_guid);
        h[72..80].copy_from_slice(&ents.to_le_bytes());
        h[80..84].copy_from_slice(&128u32.to_le_bytes());
        h[84..88].copy_from_slice(&128u32.to_le_bytes());
        h[88..92].copy_from_slice(&ecrc.to_le_bytes());
        let c = crc32(&h);
        h[16..20].copy_from_slice(&c.to_le_bytes());
        h
    };
    let mut mbr = vec![0u8; 512];
    mbr[446 + 4] = 0xEE;
    mbr[446 + 8..446 + 12].copy_from_slice(&1u32.to_le_bytes());
    mbr[446 + 12..446 + 16].copy_from_slice(&((total - 1).min(0xFFFF_FFFF) as u32).to_le_bytes());
    mbr[510] = 0x55; mbr[511] = 0xAA;
    f.seek(SeekFrom::Start(0)).unwrap(); f.write_all(&mbr).unwrap();
    f.seek(SeekFrom::Start(512)).unwrap(); f.write_all(&hdr(1, last, 2)).unwrap();
    f.seek(SeekFrom::Start(1024)).unwrap(); f.write_all(&entries).unwrap();
    f.seek(SeekFrom::Start((last - 32) * 512)).unwrap(); f.write_all(&entries).unwrap();
    f.seek(SeekFrom::Start(last * 512)).unwrap(); f.write_all(&hdr(last, 1, last - 32)).unwrap();
    f.sync_all().unwrap();
}
