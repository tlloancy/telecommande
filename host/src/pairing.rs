//! Jetons d'appairage (256 bits, un par appareil, révocables), QR, TLS auto-signé.
//! Tout vit dans ~/.config/telecommande/ en permissions 600.
use anyhow::{Context, Result};
use rand::RngCore;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::process::Command;
use std::{fs, os::unix::fs::PermissionsExt, path::{Path, PathBuf}, sync::{Arc, Mutex}};

pub struct TlsFiles {
    pub cert: PathBuf,
    pub key: PathBuf,
}

pub fn config_dir() -> Result<PathBuf> {
    let d = dirs::config_dir().context("pas de dossier de config")?.join("telecommande");
    fs::create_dir_all(&d)?;
    fs::set_permissions(&d, fs::Permissions::from_mode(0o700))?;
    Ok(d)
}

fn write_private(p: &Path, data: &[u8]) -> Result<()> {
    fs::write(p, data)?;
    fs::set_permissions(p, fs::Permissions::from_mode(0o600))?;
    Ok(())
}

pub fn hostname() -> String {
    fs::read_to_string("/etc/hostname").map(|s| s.trim().to_string()).unwrap_or_else(|_| "localhost".into())
}

/// Génère le certificat auto-signé au premier lancement.
/// Les IP du LAN sont dans le certificat : le téléphone ouvre souvent l'adresse chiffrée, pas seulement le nom .local.
pub fn ensure_tls(dir: &Path) -> Result<TlsFiles> {
    let (cert, key) = (dir.join("cert.pem"), dir.join("key.pem"));
    if !cert.exists() || !key.exists() {
        write_cert(&cert, &key)?;
    }
    Ok(TlsFiles { cert, key })
}

fn write_cert(cert_path: &Path, key_path: &Path) -> Result<()> {
    let host = hostname();
    let mut names = vec![format!("{host}.local"), host, "localhost".into()];
    names.sort();
    names.dedup();
    let mut params = rcgen::CertificateParams::new(names)?;
    params.not_before = rcgen::date_time_ymd(2026, 1, 1);
    params.not_after = rcgen::date_time_ymd(2036, 1, 1);
    params.subject_alt_names.push(rcgen::SanType::IpAddress(IpAddr::V4(Ipv4Addr::LOCALHOST)));
    params.subject_alt_names.push(rcgen::SanType::IpAddress(IpAddr::V6(Ipv6Addr::LOCALHOST)));
    for ip in preferred_ipv4() {
        params.subject_alt_names.push(rcgen::SanType::IpAddress(IpAddr::V4(ip)));
    }
    let key_pair = rcgen::KeyPair::generate()?;
    let cert = params.self_signed(&key_pair)?;
    write_private(cert_path, cert.pem().as_bytes())?;
    write_private(key_path, key_pair.serialize_pem().as_bytes())?;
    Ok(())
}

/// Lien affiché dans le QR, puis les adresses de secours.
pub fn page_urls(port: u16, token: &str) -> Vec<String> {
    let mut urls: Vec<String> = preferred_ipv4()
        .into_iter()
        .map(|ip| format!("https://{ip}:{port}/#t={token}"))
        .collect();
    let host = hostname();
    urls.push(format!("https://{host}.local:{port}/#t={token}"));
    urls.push(format!("https://127.0.0.1:{port}/#t={token}"));
    urls
}

/// Loopback, RFC1918, lien-local, et 100.64/10 (Tailscale / CGNAT).
pub fn ip_allowed(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => v.is_loopback() || v.is_private() || v.is_link_local() || is_cgnat(v),
        IpAddr::V6(v) => v.is_loopback() || v.is_unicast_link_local() || is_ula(v),
    }
}

fn is_cgnat(v: Ipv4Addr) -> bool {
    let o = v.octets();
    o[0] == 100 && (o[1] & 0xc0) == 0x40
}

fn is_ula(v: Ipv6Addr) -> bool {
    (v.segments()[0] & 0xfe00) == 0xfc00
}

fn preferred_ipv4() -> Vec<Ipv4Addr> {
    let Ok(out) = Command::new("ip").args(["-4", "-o", "addr", "show"]).output() else {
        return Vec::new();
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut scored = Vec::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let _idx = parts.next();
        let Some(iface) = parts.next() else { continue };
        let Some(kind) = parts.next() else { continue };
        if kind != "inet" {
            continue;
        }
        let Some(cidr) = parts.next() else { continue };
        let ip_str = cidr.split('/').next().unwrap_or(cidr);
        let Ok(ip) = ip_str.parse::<Ipv4Addr>() else { continue };
        if ip.is_loopback() || !ip.is_private() {
            continue;
        }
        scored.push((iface_score(iface), ip));
    }
    scored.sort_by_key(|(score, ip)| (*score, *ip));
    scored.into_iter().map(|(_, ip)| ip).collect()
}

fn iface_score(iface: &str) -> u8 {
    if iface.starts_with("wl") || iface.starts_with("en") || iface.starts_with("eth") {
        0
    } else if iface.starts_with("docker")
        || iface.starts_with("veth")
        || iface.starts_with("br-")
        || iface.starts_with("virbr")
        || iface.starts_with("tailscale")
        || iface.starts_with("tun")
    {
        2
    } else {
        1
    }
}

/// Liste de jetons stockés sur disque (hex, un par ligne). TODO phase 6 : nom d'appareil, révocation CLI, expiration.
#[derive(Clone)]
pub struct TokenStore {
    path: PathBuf,
    tokens: Arc<Mutex<Vec<String>>>,
}

impl TokenStore {
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join("tokens");
        let tokens = fs::read_to_string(&path)
            .map(|s| s.lines().map(str::to_owned).filter(|l| !l.is_empty()).collect())
            .unwrap_or_default();
        Ok(Self { path, tokens: Arc::new(Mutex::new(tokens)) })
    }

    pub fn create_token(&self) -> Result<String> {
        let mut b = [0u8; 32];
        rand::rngs::OsRng.fill_bytes(&mut b);
        let t = hex::encode(b);
        let mut g = self.tokens.lock().unwrap();
        g.push(t.clone());
        write_private(&self.path, g.join("\n").as_bytes())?;
        Ok(t)
    }

    /// Comparaison en temps constant.
    pub fn check(&self, candidate: &str) -> bool {
        self.tokens.lock().unwrap().iter().fold(false, |acc, t| {
            let eq = t.len() == candidate.len()
                && t.bytes().zip(candidate.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0;
            acc | eq
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lan_allows_private_tailscale_and_loopback() {
        assert!(ip_allowed("192.168.1.12".parse().unwrap()));
        assert!(ip_allowed("10.1.2.3".parse().unwrap()));
        assert!(ip_allowed("172.16.0.4".parse().unwrap()));
        assert!(ip_allowed("100.64.0.5".parse().unwrap()));
        assert!(ip_allowed("127.0.0.1".parse().unwrap()));
        assert!(!ip_allowed("8.8.8.8".parse().unwrap()));
        assert!(!ip_allowed("1.1.1.1".parse().unwrap()));
        assert!(ip_allowed("fd00::1".parse().unwrap()));
        assert!(!ip_allowed("2001:4860:4860::8888".parse().unwrap()));
    }
}

pub fn qr_ascii(data: &str) -> Result<String> {
    use qrcode::{render::unicode, QrCode};
    Ok(QrCode::new(data.as_bytes())?.render::<unicode::Dense1x2>().quiet_zone(true).build())
}
