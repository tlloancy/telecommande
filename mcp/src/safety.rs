//! Garde-fous : arrêt d'urgence, accord de l'utilisateur sur le bureau, journal des actions.
//!
//! - Arrêt d'urgence : le fichier `~/.config/telecommande/mcp-stop`. Tant qu'il existe, aucune
//!   action n'est exécutée, même en pleine session. `telecommande-mcp stop` le crée, `resume` le
//!   retire ; on peut lier `telecommande-mcp stop` à un raccourci clavier du bureau.
//! - Accord : la première action de la session (et toute reprise après inactivité) demande
//!   confirmation par une boîte de dialogue (zenity ou kdialog) sur le bureau lui-même.
//! - Journal : une ligne JSON par action dans `~/.local/state/telecommande/mcp-actions.log`
//!   (permissions 600). Le texte tapé n'y est jamais écrit, seulement sa longueur.
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub fn config_dir() -> PathBuf {
    let d = dirs::config_dir().unwrap_or_else(|| PathBuf::from(".")).join("telecommande");
    let _ = fs::create_dir_all(&d);
    let _ = fs::set_permissions(&d, fs::Permissions::from_mode(0o700));
    d
}

fn stop_flag() -> PathBuf {
    config_dir().join("mcp-stop")
}

pub fn is_stopped() -> bool {
    stop_flag().exists()
}

pub fn set_stopped(stop: bool) -> std::io::Result<()> {
    if stop {
        fs::write(stop_flag(), b"stop\n")
    } else {
        match fs::remove_file(stop_flag()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(()),
        }
    }
}

pub fn log_path() -> PathBuf {
    let base = dirs::state_dir()
        .or_else(dirs::data_local_dir)
        .unwrap_or_else(|| PathBuf::from("."))
        .join("telecommande");
    let _ = fs::create_dir_all(&base);
    // Le journal dit ce que l'agent a fait : le dossier n'est pas pour les autres utilisateurs.
    let _ = fs::set_permissions(&base, fs::Permissions::from_mode(0o700));
    base.join("mcp-actions.log")
}

pub struct Safety {
    idle: Duration,
    last_action: Option<Instant>,
    denied_until: Option<Instant>,
}

impl Safety {
    pub fn new(idle: Duration) -> Self {
        Self { idle, last_action: None, denied_until: None }
    }

    /// Autorise (ou non) une action. À appeler avant chaque outil qui touche au bureau.
    pub fn authorize(&mut self) -> Result<(), String> {
        if is_stopped() {
            return Err(format!(
                "Arrêt d'urgence actif ({}). L'utilisateur doit lancer `telecommande-mcp resume`.",
                stop_flag().display()
            ));
        }
        let now = Instant::now();
        if self.denied_until.is_some_and(|t| now < t) {
            return Err("L'utilisateur a refusé l'accès au bureau il y a un instant.".into());
        }
        let fresh = self.last_action.is_some_and(|t| now.duration_since(t) < self.idle);
        if !fresh {
            match ask_user() {
                Ok(true) => {}
                Ok(false) => {
                    self.denied_until = Some(now + Duration::from_secs(60));
                    return Err("L'utilisateur a refusé l'accès au bureau.".into());
                }
                Err(why) => return Err(format!("Aucune confirmation possible sur le bureau : {why}")),
            }
        }
        self.last_action = Some(now);
        Ok(())
    }
}

/// Boîte de dialogue sur le bureau. `TELECOMMANDE_MCP_APPROVE_CMD` la remplace (code 0 = accord) ;
/// `TELECOMMANDE_MCP_ASSUME_YES=1` la saute (tests, ou machine dédiée).
fn ask_user() -> Result<bool, String> {
    if std::env::var("TELECOMMANDE_MCP_ASSUME_YES").as_deref() == Ok("1") {
        return Ok(true);
    }
    let text = "Un agent IA demande à voir et contrôler ce bureau (écran, souris, clavier). Autoriser ?";
    let mut cmd = if let Ok(cmd) = std::env::var("TELECOMMANDE_MCP_APPROVE_CMD") {
        let mut c = Command::new("sh");
        c.arg("-c").arg(cmd);
        c
    } else if has("zenity") {
        let mut c = Command::new("zenity");
        c.args(["--question", "--title=Télécommande", "--ok-label=Autoriser", "--cancel-label=Refuser"])
            .arg(format!("--text={text}"))
            .arg("--timeout=60");
        c
    } else if has("kdialog") {
        let mut c = Command::new("kdialog");
        c.args(["--title", "Télécommande", "--yesno", text]);
        c
    } else {
        return Err("installe zenity ou kdialog, ou définis TELECOMMANDE_MCP_APPROVE_CMD".into());
    };
    run_with_timeout(&mut cmd, DIALOG_TIMEOUT)
}

/// Sans réponse dans ce délai, c'est un refus : une boîte oubliée ne doit pas figer le serveur.
const DIALOG_TIMEOUT: Duration = Duration::from_secs(65);

/// Ok(true) = code 0 ; Ok(false) = autre code ou délai dépassé (le processus est tué).
fn run_with_timeout(cmd: &mut Command, limit: Duration) -> Result<bool, String> {
    let mut child = cmd
        .stdin(std::process::Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status.success()),
            Ok(None) if start.elapsed() >= limit => {
                let _ = child.kill();
                let _ = child.wait();
                return Ok(false);
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(50)),
            Err(e) => return Err(e.to_string()),
        }
    }
}

fn has(bin: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(bin).is_file()))
}

/// Ajoute une ligne au journal. `detail` est du JSON déjà expurgé (pas de texte tapé).
pub fn journal(tool: &str, detail: &str, outcome: &str) {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let line = format!(
        "{{\"ts\":{secs},\"tool\":{},\"detail\":{detail},\"outcome\":{}}}\n",
        json_str(tool),
        json_str(outcome)
    );
    let file = OpenOptions::new().create(true).append(true).mode(0o600).open(log_path());
    match file {
        Ok(mut f) => {
            let _ = f.write_all(line.as_bytes());
        }
        Err(e) => tracing::warn!("journal illisible : {e}"),
    }
}

fn json_str(s: &str) -> String {
    serde_json::Value::String(s.to_string()).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dialog_exit_codes_map_to_answers() {
        let mut yes = Command::new("sh");
        yes.args(["-c", "exit 0"]);
        assert_eq!(run_with_timeout(&mut yes, Duration::from_secs(5)), Ok(true));
        let mut no = Command::new("sh");
        no.args(["-c", "exit 1"]);
        assert_eq!(run_with_timeout(&mut no, Duration::from_secs(5)), Ok(false));
    }

    #[test]
    fn hung_dialog_is_killed_and_counts_as_refusal() {
        let mut hung = Command::new("sh");
        hung.args(["-c", "sleep 30"]);
        let t = Instant::now();
        assert_eq!(run_with_timeout(&mut hung, Duration::from_millis(200)), Ok(false));
        assert!(t.elapsed() < Duration::from_secs(5));
    }
}
