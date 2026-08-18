//! TLS support for SSH and web network modes.
//!
//! Uses `rustls` to wrap the TCP listener with TLS encryption. This is
//! gated behind the `tls` cargo feature.

use std::sync::Arc;

/// Load a TLS certificate and private key from PEM files.
pub fn load_tls_config(
    cert_path: &str,
    key_path: &str,
) -> Result<Arc<rustls::ServerConfig>, Box<dyn std::error::Error>> {
    let certs = load_certs(cert_path)?;
    let key = load_private_key(key_path)?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(Arc::new(config))
}

fn load_certs(
    path: &str,
) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>, Box<dyn std::error::Error>> {
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file);
    let certs: Vec<rustls::pki_types::CertificateDer<'static>> =
        rustls_pemfile::certs(&mut reader).collect::<Result<_, _>>()?;
    Ok(certs)
}

fn load_private_key(
    path: &str,
) -> Result<rustls::pki_types::PrivateKeyDer<'static>, Box<dyn std::error::Error>> {
    let file = std::fs::File::open(path)?;
    let mut reader = std::io::BufReader::new(file);
    let keys: Vec<rustls::pki_types::PrivateKeyDer<'static>> =
        rustls_pemfile::private_key(&mut reader)?
            .into_iter()
            .collect();
    keys.into_iter()
        .next()
        .ok_or::<Box<dyn std::error::Error>>("no private key found in file".into())
}
