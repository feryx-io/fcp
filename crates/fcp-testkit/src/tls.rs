//! TLS for the test floor's clients (`docs/PRODUCTION_STACK.md` phase 3):
//! a rustls client configuration trusting one CA, and the process-level
//! crypto provider rustls 0.23 needs before any configuration is built.

use std::path::Path;
use std::sync::Arc;
use tokio_rustls::rustls::pki_types::CertificateDer;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

/// Install a default crypto provider if the process has none yet (the
/// binaries do the same; a test process built from the testkit alone
/// has not).
pub fn ensure_crypto_provider() {
    use tokio_rustls::rustls::crypto::{aws_lc_rs, CryptoProvider};
    if CryptoProvider::get_default().is_none() {
        let _ = aws_lc_rs::default_provider().install_default();
    }
}

/// The certificates in a PEM file.
pub fn read_certs(pem: &Path) -> anyhow::Result<Vec<CertificateDer<'static>>> {
    let file =
        std::fs::File::open(pem).map_err(|e| anyhow::anyhow!("open {}: {e}", pem.display()))?;
    let certs = rustls_pemfile::certs(&mut std::io::BufReader::new(file))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("read {}: {e}", pem.display()))?;
    if certs.is_empty() {
        anyhow::bail!("{}: no certificate", pem.display());
    }
    Ok(certs)
}

/// A client configuration that trusts `ca_pem` and nothing else.
pub fn client_config(ca_pem: &Path) -> anyhow::Result<Arc<ClientConfig>> {
    ensure_crypto_provider();
    let mut roots = RootCertStore::empty();
    for cert in read_certs(ca_pem)? {
        roots.add(cert)?;
    }
    Ok(Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}
