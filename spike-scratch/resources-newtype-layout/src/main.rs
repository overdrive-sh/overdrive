//! V-1 probe (#372): does wrapping `u32`/`u64` resource fields in single-field
//! newtypes change rkyv 0.8 archived bytes? If not, existing golden fixtures and
//! persisted intents decode unchanged and no envelope bump is needed.
use rkyv::{Archive, Deserialize, Serialize, rancor::Error};

#[derive(Archive, Serialize, Deserialize, Debug, PartialEq, Clone)]
struct OldResources { cpu_milli: u32, memory_bytes: u64 }

#[derive(Archive, Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
struct CpuQuantity(u32);
#[derive(Archive, Serialize, Deserialize, Debug, PartialEq, Clone, Copy)]
struct ByteSize(u64);
#[derive(Archive, Serialize, Deserialize, Debug, PartialEq, Clone)]
struct NewResources { cpu: CpuQuantity, memory: ByteSize }

// Embedded in a larger aggregate with an enum envelope, like WorkloadIntentEnvelope::V1.
#[derive(Archive, Serialize, Deserialize, Debug, PartialEq)]
struct OldJob { id: String, replicas: u32, resources: OldResources, tail: Option<u64> }
#[derive(Archive, Serialize, Deserialize, Debug, PartialEq)]
struct NewJob { id: String, replicas: u32, resources: NewResources, tail: Option<u64> }
#[derive(Archive, Serialize, Deserialize, Debug, PartialEq)]
enum OldEnv { V1(OldJob) }
#[derive(Archive, Serialize, Deserialize, Debug, PartialEq)]
enum NewEnv { V1(NewJob) }

fn hex(b: &[u8]) -> String { b.iter().map(|x| format!("{x:02x}")).collect() }

fn main() {
    let mut ok = true;
    for (c, m) in [(0u32, 1u64), (500, 134_217_728), (u32::MAX, u64::MAX), (1500, 1 << 40)] {
        let o = rkyv::to_bytes::<Error>(&OldResources { cpu_milli: c, memory_bytes: m }).unwrap();
        let n = rkyv::to_bytes::<Error>(&NewResources { cpu: CpuQuantity(c), memory: ByteSize(m) }).unwrap();
        let eq = o.as_slice() == n.as_slice();
        ok &= eq;
        println!("flat c={c} m={m}: old={} new={} equal={eq}", hex(&o), hex(&n));

        let oj = OldEnv::V1(OldJob { id: "payments".into(), replicas: 3, resources: OldResources { cpu_milli: c, memory_bytes: m }, tail: Some(7) });
        let nj = NewEnv::V1(NewJob { id: "payments".into(), replicas: 3, resources: NewResources { cpu: CpuQuantity(c), memory: ByteSize(m) }, tail: Some(7) });
        let ob = rkyv::to_bytes::<Error>(&oj).unwrap();
        let nb = rkyv::to_bytes::<Error>(&nj).unwrap();
        let eq2 = ob.as_slice() == nb.as_slice();
        // Cross-decode: old bytes through the new (validated) envelope type.
        let decoded = rkyv::from_bytes::<NewEnv, Error>(&ob).expect("old bytes must validate+decode as new type");
        let NewEnv::V1(j) = decoded;
        let cross = j.resources.cpu.0 == c && j.resources.memory.0 == m && j.id == "payments" && j.tail == Some(7);
        ok &= eq2 && cross;
        println!("envelope c={c} m={m}: len old={} new={} equal={eq2} cross_decode={cross}", ob.len(), nb.len());
    }
    println!("VERDICT: {}", if ok { "WORKS (byte-identical, cross-decodes)" } else { "DOESN'T-WORK" });
    std::process::exit(if ok { 0 } else { 1 });
}
