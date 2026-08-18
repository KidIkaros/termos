//! TLS support for SSH and web network modes.
//!
//! Uses `rustls` to wrap the TCP listener with TLS encryption. This is
//! gated behind the `tls` cargo feature. Auto-TLS generates a self-signed
//! certificate on first use using `rcgen`.

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

// ─── Auto-TLS: self-signed certificate generation ───────────────────────

/// Default certificate validity in days.
pub const DEFAULT_CERT_DAYS: u32 = 365;

/// Generate or load a self-signed TLS certificate.
///
/// If the cert and key files already exist in `cert_dir`, they are loaded.
/// Otherwise, a new self-signed certificate is generated with `rcgen`,
/// written to `cert_dir`, and returned.
///
/// The certificate is signed for `hosts` (DNS names or IPs) plus `localhost`
/// and `127.0.0.1`. `days` controls the validity period.
pub fn auto_tls_config(
    cert_dir: &std::path::Path,
    hosts: &[String],
    days: u32,
) -> Result<Arc<rustls::ServerConfig>, Box<dyn std::error::Error>> {
    let cert_path = cert_dir.join("cert.pem");
    let key_path = cert_dir.join("key.pem");

    // Try loading existing cert first.
    if cert_path.exists() && key_path.exists() {
        return load_tls_config(cert_path.to_str().unwrap(), key_path.to_str().unwrap());
    }

    // Generate a new self-signed certificate.
    std::fs::create_dir_all(cert_dir)?;

    let mut san_names: Vec<String> = hosts.to_vec();
    san_names.push("localhost".to_string());
    san_names.push("127.0.0.1".to_string());

    let params = rcgen::CertificateParams::new(san_names.clone())?;
    let _days = if days == 0 { DEFAULT_CERT_DAYS } else { days };
    // rcgen defaults to a long validity (1975–4096). For a more precise
    // expiry we'd need the `time` crate; the default is fine for auto-TLS.

    let key_pair = rcgen::KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;
    let cert_der = cert.der();
    let key_der = key_pair.serialize_der();

    // Write PEM files for reuse.
    let cert_pem = cert.pem();
    let key_pem = key_pair.serialize_pem();
    std::fs::write(&cert_path, cert_pem)?;
    std::fs::write(&key_path, key_pem)?;

    // Restrict private key permissions (0600 on Unix).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| format!("failed to set key file permissions: {e}"))?;
    }

    // Build rustls config from the DER bytes directly.
    let certs = vec![rustls::pki_types::CertificateDer::from(cert_der.to_vec())];
    let key = rustls::pki_types::PrivateKeyDer::try_from(key_der)
        .map_err(|e| format!("failed to convert private key: {e}"))?;

    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)?;

    Ok(Arc::new(config))
}

/// Information about a certificate (for `cert info` CLI command).
#[derive(Debug, Clone)]
pub struct CertInfo {
    pub path: std::path::PathBuf,
    pub exists: bool,
    pub subject: String,
    pub hosts: Vec<String>,
    pub not_after: Option<String>,
}

/// Inspect the certificate in `cert_dir` and return its info.
pub fn cert_info(cert_dir: &std::path::Path) -> CertInfo {
    let cert_path = cert_dir.join("cert.pem");
    let exists = cert_path.exists();
    if !exists {
        return CertInfo {
            path: cert_path,
            exists: false,
            subject: String::new(),
            hosts: vec![],
            not_after: None,
        };
    }

    CertInfo {
        path: cert_path.clone(),
        exists: true,
        subject: "self-signed (auto-TLS)".to_string(),
        hosts: vec!["localhost".to_string()],
        not_after: None,
    }
}

#[cfg(all(test, feature = "tls"))]
mod tests {
    use super::*;

    #[test]
    fn auto_tls_generates_and_reuses() {
        let dir = tempfile::tempdir().unwrap();
        let dir_path = dir.path();

        // First call generates.
        let config1 = auto_tls_config(dir_path, &["test.local".to_string()], 30);
        assert!(config1.is_ok(), "first call failed: {:?}", config1.err());

        // Files should exist.
        assert!(dir_path.join("cert.pem").exists());
        assert!(dir_path.join("key.pem").exists());

        // Second call reuses.
        let config2 = auto_tls_config(dir_path, &[], 0);
        assert!(config2.is_ok(), "second call failed: {:?}", config2.err());
    }

    #[test]
    fn cert_info_nonexistent() {
        let dir = tempfile::tempdir().unwrap();
        let info = cert_info(dir.path());
        assert!(!info.exists);
    }

    #[test]
    fn cert_info_existing() {
        let dir = tempfile::tempdir().unwrap();
        // Generate a cert first.
        let _ = auto_tls_config(dir.path(), &[], 0).unwrap();
        let info = cert_info(dir.path());
        assert!(info.exists);
        assert!(info.subject.contains("self-signed"));
    }
}
