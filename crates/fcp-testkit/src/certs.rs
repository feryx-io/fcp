//! A throwaway certificate authority for a test cluster: inter-node
//! conference trunks are mutual TLS, and a node is admitted as a trunk
//! peer only when its client certificate names its node id
//! (`docs/DISTRIBUTED_CONFERENCING.md`, phase 2). The CA and each node's
//! certificate and key are written as PEM files, the keys `0600` as the
//! loaders require.

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

/// A CA on disk, minting node certificates.
pub struct ClusterCa {
    dir: PathBuf,
    key: rcgen::KeyPair,
    cert: rcgen::Certificate,
    /// The CA certificate, PEM: what every node trusts its peers against.
    pub ca_path: PathBuf,
}

/// One node's certificate and key on disk.
#[derive(Debug, Clone)]
pub struct NodeCert {
    pub cert_path: PathBuf,
    pub key_path: PathBuf,
}

impl ClusterCa {
    /// A new CA under `dir`.
    pub fn new(dir: &Path) -> Self {
        let key = rcgen::KeyPair::generate().expect("CA key");
        let mut params = rcgen::CertificateParams::default();
        params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, "fcp test cluster CA");
        let cert = params.self_signed(&key).expect("CA certificate");
        let ca_path = dir.join("cluster-ca.pem");
        write(&ca_path, &cert.pem(), 0o644);
        Self {
            dir: dir.to_path_buf(),
            key,
            cert,
            ca_path,
        }
    }

    /// A certificate naming `node_id` (CN and DNS SAN, plus the loopback
    /// address it is dialed at), signed by this CA.
    pub fn node(&self, node_id: &str) -> NodeCert {
        let key = rcgen::KeyPair::generate().expect("node key");
        let mut params = rcgen::CertificateParams::new(vec![node_id.to_string()]).expect("params");
        params.distinguished_name = rcgen::DistinguishedName::new();
        params
            .distinguished_name
            .push(rcgen::DnType::CommonName, node_id);
        params
            .subject_alt_names
            .push(rcgen::SanType::IpAddress("127.0.0.1".parse().unwrap()));
        let cert = params
            .signed_by(&key, &self.cert, &self.key)
            .expect("node certificate");
        let cert_path = self.dir.join(format!("{node_id}.crt"));
        let key_path = self.dir.join(format!("{node_id}.key"));
        write(&cert_path, &cert.pem(), 0o644);
        write(&key_path, &key.serialize_pem(), 0o600);
        NodeCert {
            cert_path,
            key_path,
        }
    }
}

fn write(path: &Path, contents: &str, mode: u32) {
    std::fs::write(path, contents).unwrap_or_else(|e| panic!("write {}: {e}", path.display()));
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode))
        .unwrap_or_else(|e| panic!("chmod {}: {e}", path.display()));
}
