//! Jetons d'appairage (256 bits, un par appareil, révocables), QR, TLS auto-signé.
//! Tout vit dans ~/.config/telecommande/ en permissions 600.
use anyhow::{Context, Result};
use rand::RngCore;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::process::Command;
use std::time::{Duration, Instant, SystemTime};
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
/// `frag` vaut `p=<code>` (code d'appairage court) ou `t=<jeton>` (`--pair`, jeton permanent).
pub fn page_urls(port: u16, frag: &str) -> Vec<String> {
    let mut urls: Vec<String> = preferred_ipv4()
        .into_iter()
        .map(|ip| format!("https://{ip}:{port}/#{frag}"))
        .collect();
    let host = hostname();
    urls.push(format!("https://{host}.local:{port}/#{frag}"));
    urls.push(format!("https://127.0.0.1:{port}/#{frag}"));
    urls
}

/// Affiche le QR et les adresses de secours dans le terminal (et le journal systemd).
pub fn print_qr(port: u16, frag: &str, valid_for: Option<Duration>) -> Result<()> {
    let urls = page_urls(port, frag);
    let url = urls.first().cloned().unwrap_or_default();
    let validity = match valid_for {
        Some(d) => format!(" — valable {} min {:02} s, usage unique", d.as_secs() / 60, d.as_secs() % 60),
        None => String::new(),
    };
    println!("\nScanne ce QR avec le téléphone (même Wi-Fi){validity} :\n{}\n{url}\n", qr_ascii(&url)?);
    for extra in urls.iter().skip(1) {
        println!("Autre adresse : {extra}");
    }
    Ok(())
}

fn random_hex<const N: usize>() -> String {
    let mut b = [0u8; N];
    rand::rngs::OsRng.fill_bytes(&mut b);
    hex::encode(b)
}

/// Comparaison en temps constant.
fn ct_eq(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Essais ratés tolérés par adresse IP sur `FAILURE_WINDOW`.
const MAX_FAILURES: u32 = 5;
const FAILURE_WINDOW: Duration = Duration::from_secs(60);

#[derive(Debug, PartialEq)]
pub enum RedeemError {
    /// Code juste remplacé : le téléphone a scanné un QR périmé.
    Expired,
    Invalid,
    RateLimited,
}

struct PendingCode {
    code: String,
    expires: Instant,
    /// Code précédent, gardé pour dire « expiré » plutôt que « refusé ».
    previous: Option<String>,
}

/// Code d'appairage du QR : 128 bits, usage unique, durée limitée, renouvelé seul.
/// Il ne donne aucun accès : il s'échange une fois contre un jeton d'appareil (`TokenStore`).
#[derive(Clone)]
pub struct Pairing {
    ttl: Duration,
    pending: Arc<Mutex<PendingCode>>,
    failures: Arc<Mutex<HashMap<IpAddr, (u32, Instant)>>>,
    /// Réveille l'affichage du QR quand le code change avant son expiration.
    pub changed: Arc<tokio::sync::Notify>,
}

impl Pairing {
    pub fn new(ttl: Duration) -> Self {
        let pending = PendingCode { code: random_hex::<16>(), expires: Instant::now() + ttl, previous: None };
        Self {
            ttl,
            pending: Arc::new(Mutex::new(pending)),
            failures: Arc::new(Mutex::new(HashMap::new())),
            changed: Arc::new(tokio::sync::Notify::new()),
        }
    }

    fn rotate_locked(&self, p: &mut PendingCode) {
        let old = std::mem::replace(&mut p.code, random_hex::<16>());
        p.previous = Some(old);
        p.expires = Instant::now() + self.ttl;
    }

    /// Code en cours et temps restant ; un code expiré est remplacé ici.
    pub fn current(&self) -> (String, Duration) {
        let mut p = self.pending.lock().unwrap();
        if Instant::now() >= p.expires {
            self.rotate_locked(&mut p);
        }
        (p.code.clone(), p.expires.saturating_duration_since(Instant::now()))
    }

    /// Valide le code une seule fois, puis le remplace (et réaffiche un nouveau QR).
    pub fn redeem(&self, ip: IpAddr, candidate: &str) -> Result<(), RedeemError> {
        let now = Instant::now();
        {
            let mut f = self.failures.lock().unwrap();
            f.retain(|_, (_, since)| now.duration_since(*since) < FAILURE_WINDOW);
            if f.get(&ip).is_some_and(|(n, _)| *n >= MAX_FAILURES) {
                return Err(RedeemError::RateLimited);
            }
        }
        let outcome = {
            let mut p = self.pending.lock().unwrap();
            if now >= p.expires {
                self.rotate_locked(&mut p);
            }
            if ct_eq(&p.code, candidate) {
                self.rotate_locked(&mut p);
                p.previous = None;
                Ok(())
            } else if p.previous.as_deref().is_some_and(|old| ct_eq(old, candidate)) {
                Err(RedeemError::Expired)
            } else {
                Err(RedeemError::Invalid)
            }
        };
        match &outcome {
            Ok(()) => self.changed.notify_one(),
            Err(_) => {
                let mut f = self.failures.lock().unwrap();
                f.entry(ip).or_insert((0, now)).0 += 1;
            }
        }
        outcome
    }
}

/// Réaffiche le QR à chaque renouvellement : le code n'est jamais mort à l'écran.
pub async fn announce_loop(pairing: Pairing, port: u16) {
    loop {
        let (code, left) = pairing.current();
        if let Err(err) = print_qr(port, &format!("p={code}"), Some(left)) {
            tracing::warn!("QR : {err:#}");
        }
        tokio::select! {
            _ = tokio::time::sleep(left) => {}
            _ = pairing.changed.notified() => {}
        }
    }
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
    mtime: Arc<Mutex<Option<SystemTime>>>,
}

fn read_tokens(path: &Path) -> Vec<String> {
    fs::read_to_string(path)
        .map(|s| s.lines().map(str::to_owned).filter(|l| !l.is_empty()).collect())
        .unwrap_or_default()
}

fn mtime(path: &Path) -> Option<SystemTime> {
    fs::metadata(path).and_then(|m| m.modified()).ok()
}

impl TokenStore {
    pub fn load(dir: &Path) -> Result<Self> {
        let path = dir.join("tokens");
        let tokens = read_tokens(&path);
        let stamp = mtime(&path);
        Ok(Self { path, tokens: Arc::new(Mutex::new(tokens)), mtime: Arc::new(Mutex::new(stamp)) })
    }

    pub fn create_token(&self) -> Result<String> {
        let t = random_hex::<32>();
        let mut g = self.tokens.lock().unwrap();
        g.push(t.clone());
        write_private(&self.path, g.join("\n").as_bytes())?;
        *self.mtime.lock().unwrap() = mtime(&self.path);
        Ok(t)
    }

    fn matches(&self, candidate: &str) -> bool {
        self.tokens.lock().unwrap().iter().fold(false, |acc, t| acc | ct_eq(t, candidate))
    }

    /// Comparaison en temps constant. `--pair` lancé à côté du service écrit le fichier :
    /// on le relit (seulement s'il a changé) avant de refuser.
    pub fn check(&self, candidate: &str) -> bool {
        if self.matches(candidate) {
            return true;
        }
        let now = mtime(&self.path);
        let mut seen = self.mtime.lock().unwrap();
        if now != *seen {
            *seen = now;
            *self.tokens.lock().unwrap() = read_tokens(&self.path);
            drop(seen);
            return self.matches(candidate);
        }
        false
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

    #[test]
    fn pairing_code_is_single_use() {
        let p = Pairing::new(Duration::from_secs(300));
        let ip: IpAddr = "192.168.1.12".parse().unwrap();
        let (code, left) = p.current();
        assert!(left > Duration::from_secs(290));
        assert_eq!(p.redeem(ip, &code), Ok(()));
        assert_eq!(p.redeem(ip, &code), Err(RedeemError::Invalid));
        assert_ne!(p.current().0, code);
    }

    #[test]
    fn expired_code_is_replaced_and_reported() {
        let p = Pairing::new(Duration::from_millis(1));
        let ip: IpAddr = "192.168.1.12".parse().unwrap();
        let (code, _) = p.current();
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(p.redeem(ip, &code), Err(RedeemError::Expired));
    }

    #[test]
    fn failures_are_rate_limited_per_ip() {
        let p = Pairing::new(Duration::from_secs(300));
        let bad: IpAddr = "192.168.1.66".parse().unwrap();
        for _ in 0..MAX_FAILURES {
            assert_eq!(p.redeem(bad, "00"), Err(RedeemError::Invalid));
        }
        let (code, _) = p.current();
        assert_eq!(p.redeem(bad, &code), Err(RedeemError::RateLimited));
        assert_eq!(p.redeem("192.168.1.12".parse().unwrap(), &code), Ok(()));
    }
}

pub fn qr_ascii(data: &str) -> Result<String> {
    use qrcode::{render::unicode, QrCode};
    Ok(QrCode::new(data.as_bytes())?.render::<unicode::Dense1x2>().quiet_zone(true).build())
}
