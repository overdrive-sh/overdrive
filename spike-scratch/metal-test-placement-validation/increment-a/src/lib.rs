//! PROBE V-1 support: read the `overdrive_test_placement` section of the running
//! test binary (64-bit LE ELF) and return its records.
use std::fs;

pub fn placement_records() -> Vec<(String, String)> {
    let path = std::env::current_exe().expect("current_exe");
    let bytes = fs::read(&path).expect("read own binary");
    let u16at = |o: usize| u16::from_le_bytes(bytes[o..o + 2].try_into().unwrap()) as usize;
    let u32at = |o: usize| u32::from_le_bytes(bytes[o..o + 4].try_into().unwrap()) as usize;
    let u64at = |o: usize| u64::from_le_bytes(bytes[o..o + 8].try_into().unwrap()) as usize;
    let (table, entry, count, names) = (u64at(40), u16at(58), u16at(60), u16at(62));
    let header = |i: usize| table + i * entry;
    let strtab = u64at(header(names) + 24);
    let mut records = Vec::new();
    for i in 0..count {
        let h = header(i);
        let name_at = strtab + u32at(h);
        let end = bytes[name_at..].iter().position(|b| *b == 0).unwrap();
        if &bytes[name_at..name_at + end] != b"overdrive_test_placement" {
            continue;
        }
        let (off, size) = (u64at(h + 24), u64at(h + 32));
        let mut parts = bytes[off..off + size].split(|b| *b == 0);
        while let (Some(name), Some(group)) = (parts.next(), parts.next()) {
            if name.is_empty() {
                continue;
            }
            records.push((String::from_utf8_lossy(name).into(), String::from_utf8_lossy(group).into()));
        }
    }
    records
}

pub fn dump(label: &str) {
    let records = placement_records();
    println!("SECTION_DUMP {label} records={}", records.len());
    for (name, group) in records {
        println!("SECTION_RECORD name={name} group={group}");
    }
}
