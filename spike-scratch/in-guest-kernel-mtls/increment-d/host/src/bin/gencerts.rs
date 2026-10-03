//! Spike B: throwaway test PKI (GH #303, increment-d).
//!
//!   gencerts <out-dir>
//!
//! One test CA and six ECDSA P-256 leaves, each with one `spiffe://` URI SAN:
//!   client        spiffe://overdrive.test/ns/default/sa/guest-client  (clientAuth; held by the relay stub)
//!   peer-allowed  spiffe://overdrive.test/ns/default/sa/peer-allowed  (serverAuth, IP SAN 192.168.203.1)
//!   peer-denied   spiffe://overdrive.test/ns/default/sa/peer-denied   (serverAuth, IP SAN 192.168.203.1)
//!   guest-server  spiffe://overdrive.test/ns/default/sa/guest-server  (serverAuth, IP SAN 192.168.203.2; held by the relay)
//!   peer-client   spiffe://overdrive.test/ns/default/sa/peer-client   (clientAuth; the allowed inbound caller)
//!   peer-client-denied spiffe://overdrive.test/ns/default/sa/peer-client-denied (clientAuth; the denied inbound caller)
//! Validity 2026-09-27 .. 2026-10-28 — a window that excludes the guest's
//! 1999-12-31 wall clock, so a guest-side validity check would reject it.
use std::fs;
use std::net::{IpAddr, Ipv4Addr};

use rcgen::string::Ia5String;
use rcgen::{
    date_time_ymd, BasicConstraints, CertificateParams, DistinguishedName, DnType,
    ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair, KeyUsagePurpose, SanType,
};

const TRUST_DOMAIN: &str = "spiffe://overdrive.test/ns/default/sa/";

fn base(cn: &str) -> CertificateParams {
    let mut p = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::CommonName, cn);
    p.distinguished_name = dn;
    p.not_before = date_time_ymd(2026, 9, 27);
    p.not_after = date_time_ymd(2026, 10, 28);
    p.use_authority_key_identifier_extension = true;
    p
}

fn write(dir: &str, name: &str, cert_pem: &str, key: &KeyPair) {
    fs::write(format!("{dir}/{name}.pem"), cert_pem).unwrap();
    fs::write(format!("{dir}/{name}.key"), key.serialize_pem()).unwrap();
}

fn main() {
    let dir = std::env::args().nth(1).expect("usage: gencerts <out-dir>");
    fs::create_dir_all(&dir).unwrap();

    let ca_key = KeyPair::generate().unwrap();
    let mut ca = base("igkm-d spike test CA");
    ca.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
    ca.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign];
    let ca_cert = ca.self_signed(&ca_key).unwrap();
    write(&dir, "ca", &ca_cert.pem(), &ca_key);
    let issuer = Issuer::new(ca, ca_key);

    let leaves: [(&str, &str, bool, u8); 6] = [
        ("client", "guest-client", false, 0),
        ("peer-allowed", "peer-allowed", true, 1),
        ("peer-denied", "peer-denied", true, 1),
        ("guest-server", "guest-server", true, 2),
        ("peer-client", "peer-client", false, 0),
        ("peer-client-denied", "peer-client-denied", false, 0),
    ];
    for (file, sa, server, ip_last) in leaves {
        let key = KeyPair::generate().unwrap();
        let mut p = base(&format!("igkm-d {sa}"));
        let uri = format!("{TRUST_DOMAIN}{sa}");
        p.subject_alt_names = vec![SanType::URI(Ia5String::try_from(uri.clone()).unwrap())];
        if server {
            p.subject_alt_names
                .push(SanType::IpAddress(IpAddr::V4(Ipv4Addr::new(192, 168, 203, ip_last))));
        }
        p.key_usages = vec![KeyUsagePurpose::DigitalSignature];
        p.extended_key_usages = vec![if server {
            ExtendedKeyUsagePurpose::ServerAuth
        } else {
            ExtendedKeyUsagePurpose::ClientAuth
        }];
        let cert = p.signed_by(&key, &issuer).unwrap();
        write(&dir, file, &cert.pem(), &key);
        let got = igkmd_host::spiffe_id(cert.der()).unwrap();
        println!(
            "GENCERTS: {file}.pem spiffe={got} server={server} validity=2026-09-27..2026-10-28 der_len={}",
            cert.der().len()
        );
    }
    println!("GENCERTS: ca.pem CN=igkm-d spike test CA");
}
