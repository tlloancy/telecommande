//! Serveur MCP local (stdio) pour piloter le bureau X11 de cette machine.
//!
//! Lancé par l'agent (Grok Build, Cursor...) comme un sous-processus : aucun port ouvert, aucun
//! réseau. Chaque action passe par `safety` (arrêt d'urgence, accord sur le bureau, journal).
//!
//!   telecommande-mcp            serveur stdio
//!   telecommande-mcp stop       arrêt d'urgence : plus aucune action tant que `resume` n'est pas lancé
//!   telecommande-mcp resume     lève l'arrêt d'urgence
//!   telecommande-mcp status     état de l'arrêt d'urgence et chemin du journal
#[allow(dead_code)]
#[path = "../../host/src/keyboard.rs"]
mod keyboard;
#[allow(dead_code)]
#[path = "../../host/src/screens.rs"]
mod screens;

mod keys;
mod safety;
mod shot;
mod tools;

use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::time::Duration;

const SUPPORTED: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];

fn main() -> anyhow::Result<()> {
    // stdout porte le protocole : les logs vont sur stderr.
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .with_writer(std::io::stderr)
        .init();

    let mut idle = 600u64;
    let mut command = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--idle-timeout" => {
                idle = args.next().and_then(|v| v.parse().ok()).unwrap_or(idle).clamp(30, 86_400);
            }
            "stop" | "resume" | "status" => command = Some(a),
            "-h" | "--help" => {
                println!("{}", include_str!("help.txt"));
                return Ok(());
            }
            other => anyhow::bail!("argument inconnu : {other} (voir --help)"),
        }
    }
    match command.as_deref() {
        Some("stop") => {
            safety::set_stopped(true)?;
            println!("Arrêt d'urgence activé : l'agent ne peut plus rien faire sur ce bureau.");
            Ok(())
        }
        Some("resume") => {
            safety::set_stopped(false)?;
            println!("Arrêt d'urgence levé.");
            Ok(())
        }
        Some(_) => {
            println!("Arrêt d'urgence : {}", if safety::is_stopped() { "ACTIF" } else { "inactif" });
            println!("Journal : {}", safety::log_path().display());
            Ok(())
        }
        None => serve(Duration::from_secs(idle)),
    }
}

fn serve(idle: Duration) -> anyhow::Result<()> {
    let mut tools = tools::Tools::new(safety::Safety::new(idle));
    let stdin = std::io::stdin();
    let mut out = std::io::stdout().lock();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let Ok(msg) = serde_json::from_str::<Value>(&line) else {
            write(&mut out, &error(Value::Null, -32700, "JSON invalide"))?;
            continue;
        };
        if let Some(reply) = handle(&mut tools, &msg) {
            write(&mut out, &reply)?;
        }
    }
    Ok(())
}

fn write(out: &mut impl Write, v: &Value) -> std::io::Result<()> {
    writeln!(out, "{v}")?;
    out.flush()
}

fn error(id: Value, code: i32, message: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}})
}

/// Une réponse pour les requêtes (avec `id`), rien pour les notifications.
fn handle(tools: &mut tools::Tools, msg: &Value) -> Option<Value> {
    let id = msg.get("id").cloned()?;
    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(Value::Null);
    let result = match method {
        "initialize" => {
            let asked = params.get("protocolVersion").and_then(Value::as_str).unwrap_or("");
            let version = if SUPPORTED.contains(&asked) { asked } else { SUPPORTED[0] };
            json!({
                "protocolVersion": version,
                "capabilities": {"tools": {}},
                "serverInfo": {"name": "telecommande-mcp", "version": env!("CARGO_PKG_VERSION")},
                "instructions": "Pilote le bureau de l'utilisateur (capture, souris, clavier). Chaque session demande son accord sur le bureau ; l'utilisateur peut tout couper avec `telecommande-mcp stop`. Commence par screenshot : les coordonnées sont celles de la dernière capture."
            })
        }
        "ping" => json!({}),
        "tools/list" => json!({"tools": tools::definitions()}),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
            match tools.call(name, &args) {
                Ok(content) => json!({"content": content}),
                Err(why) => json!({"content": [{"type": "text", "text": why}], "isError": true}),
            }
        }
        _ => return Some(error(id, -32601, "méthode inconnue")),
    };
    Some(json!({"jsonrpc": "2.0", "id": id, "result": result}))
}
