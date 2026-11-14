//! HTTPS (rustls) : sert la PWA et la signalisation WebSocket, authentifiée par jeton.
//! Les entrées ne passent pas par ce socket : elles arrivent sur les DataChannels, après l'échange SDP.

use crate::{
    capture::{self, CaptureOpts, HostSignal},
    encoder::Encoder,
    pairing::{self, Pairing, RedeemError, TlsFiles, TokenStore},
};
use axum::{
    body::Body,
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        ConnectInfo, Query, State,
    },
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use axum_server::tls_rustls::RustlsConfig;
use futures_util::{SinkExt, StreamExt};
use serde::Deserialize;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tower_http::services::ServeDir;

pub struct Config {
    pub pairing: Pairing,
    pub port: u16,
    pub web_dir: PathBuf,
    pub tls: TlsFiles,
    pub store: TokenStore,
    pub encoder: Encoder,
    pub capture: CaptureOpts,
    pub listen_all: bool,
}

#[derive(Clone)]
struct AppState {
    pairing: Pairing,
    store: TokenStore,
    encoder: Encoder,
    capture: CaptureOpts,
    listen_all: bool,
}

#[derive(Deserialize)]
struct WsAuth {
    token: String,
}

#[derive(Deserialize)]
struct PairRequest {
    code: String,
}

pub async fn run(cfg: Config) -> anyhow::Result<()> {
    let state = AppState {
        pairing: cfg.pairing,
        store: cfg.store,
        encoder: cfg.encoder,
        capture: cfg.capture,
        listen_all: cfg.listen_all,
    };
    let app = Router::new()
        .route("/ws", get(ws_handler))
        .route("/pair", post(pair_handler))
        .fallback_service(ServeDir::new(&cfg.web_dir))
        .layer(middleware::from_fn_with_state(state.clone(), lan_gate))
        .with_state(state);

    let tls = RustlsConfig::from_pem_file(&cfg.tls.cert, &cfg.tls.key).await?;
    let addr = SocketAddr::from(([0, 0, 0, 0], cfg.port));
    tracing::info!("écoute sur https://{addr} (listen_all={})", cfg.listen_all);
    axum_server::bind_rustls(addr, tls)
        .serve(app.into_make_service_with_connect_info::<SocketAddr>())
        .await?;
    Ok(())
}

async fn lan_gate(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    req: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    if state.listen_all || pairing::ip_allowed(peer.ip()) {
        Ok(next.run(req).await)
    } else {
        tracing::warn!("connexion refusée hors LAN : {}", peer.ip());
        Err(StatusCode::FORBIDDEN)
    }
}

async fn ws_handler(
    ws: WebSocketUpgrade,
    Query(auth): Query<WsAuth>,
    State(state): State<AppState>,
) -> impl IntoResponse {
    if !state.store.check(&auth.token) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let encoder = state.encoder;
    let capture = state.capture;
    ws.on_upgrade(move |sock| handle_socket(sock, encoder, capture))
        .into_response()
}

/// Échange le code du QR (usage unique, durée limitée) contre un jeton d'appareil permanent.
async fn pair_handler(
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    State(state): State<AppState>,
    Json(req): Json<PairRequest>,
) -> Response {
    match state.pairing.redeem(peer.ip(), &req.code) {
        Ok(()) => match state.store.create_token() {
            Ok(token) => {
                tracing::info!("nouvel appareil appairé depuis {}", peer.ip());
                Json(serde_json::json!({ "token": token })).into_response()
            }
            Err(err) => {
                tracing::error!("jeton non enregistré : {err:#}");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        },
        Err(RedeemError::Expired) => StatusCode::GONE.into_response(),
        Err(RedeemError::RateLimited) => {
            tracing::warn!("appairage : trop d'essais depuis {}", peer.ip());
            StatusCode::TOO_MANY_REQUESTS.into_response()
        }
        Err(RedeemError::Invalid) => StatusCode::FORBIDDEN.into_response(),
    }
}

enum ClientCmd {
    Text(String),
    Stop,
}

/// Un téléphone qui change de borne Wi-Fi disparaît sans fermer le WebSocket : sans ping, la
/// session (capture + encodage) tournerait jusqu'au délai TCP, souvent plusieurs minutes.
const PING_EVERY: Duration = Duration::from_secs(10);
const DEAD_AFTER: Duration = Duration::from_secs(30);

struct Heartbeat {
    last_seen: Instant,
}

impl Heartbeat {
    fn new(now: Instant) -> Self {
        Self { last_seen: now }
    }

    /// Tout message reçu (texte, pong...) prouve que le téléphone est là.
    fn seen(&mut self, now: Instant) {
        self.last_seen = now;
    }

    fn dead(&self, now: Instant) -> bool {
        now.duration_since(self.last_seen) > DEAD_AFTER
    }
}

async fn handle_socket(sock: WebSocket, encoder: Encoder, opts: CaptureOpts) {
    let (mut sink, mut stream) = sock.split();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    // La session GStreamer n'est pas Send : elle vit dans son thread, le WebSocket reste async.
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel();
    let tx_fail = tx.clone();
    std::thread::Builder::new()
        .name("session".into())
        .spawn(move || run_session(encoder, opts, tx, tx_fail, cmd_rx))
        .expect("thread session");

    let mut beat = Heartbeat::new(Instant::now());
    let mut ticker = tokio::time::interval_at(tokio::time::Instant::now() + PING_EVERY, PING_EVERY);
    loop {
        tokio::select! {
            _ = ticker.tick() => {
                if beat.dead(Instant::now()) {
                    tracing::info!("session abandonnée : plus aucun signe du téléphone");
                    break;
                }
                if sink.send(Message::Ping(Vec::new())).await.is_err() {
                    break;
                }
            }
            outgoing = rx.recv() => {
                let Some(signal) = outgoing else { break };
                if sink.send(Message::Text(signal_text(&signal).into())).await.is_err() {
                    break;
                }
            }
            incoming = stream.next() => {
                beat.seen(Instant::now());
                match incoming {
                    Some(Ok(Message::Text(text))) => {
                        if cmd_tx.send(ClientCmd::Text(text)).is_err() {
                            break;
                        }
                    }
                    Some(Ok(Message::Close(_))) | None => break,
                    Some(Ok(_)) => {}
                    Some(Err(_)) => break,
                }
            }
        }
    }
    let _ = cmd_tx.send(ClientCmd::Stop);
}

fn run_session(
    encoder: Encoder,
    opts: CaptureOpts,
    tx: tokio::sync::mpsc::UnboundedSender<HostSignal>,
    tx_fail: tokio::sync::mpsc::UnboundedSender<HostSignal>,
    cmd_rx: std::sync::mpsc::Receiver<ClientCmd>,
) {
    let session = match capture::Session::start(encoder, opts, tx) {
        Ok(session) => session,
        Err(err) => {
            tracing::error!("{err:#}");
            let _ = tx_fail.send(HostSignal::Failed);
            return;
        }
    };
    while let Ok(cmd) = cmd_rx.recv() {
        match cmd {
            ClientCmd::Text(text) => {
                if let Err(err) = session.on_client_text(&text) {
                    tracing::warn!("signalisation : {err:#}");
                }
            }
            ClientCmd::Stop => break,
        }
    }
    session.stop();
}

fn signal_text(signal: &HostSignal) -> String {
    match signal {
        HostSignal::Offer(sdp) => serde_json::json!({"kind": "offer", "sdp": sdp}).to_string(),
        HostSignal::Ice { mline, candidate } => serde_json::json!({
            "kind": "ice",
            "candidate": candidate,
            "sdpMLineIndex": mline,
        })
        .to_string(),
        HostSignal::Failed => error_text("La capture de l'écran s'est arrêtée."),
    }
}

fn error_text(message: &str) -> String {
    serde_json::json!({"kind": "error", "message": message}).to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_client_is_dead_only_after_the_delay() {
        let t0 = Instant::now();
        let beat = Heartbeat::new(t0);
        assert!(!beat.dead(t0 + Duration::from_secs(29)));
        assert!(beat.dead(t0 + Duration::from_secs(31)));
    }

    #[test]
    fn any_message_resets_the_delay() {
        let t0 = Instant::now();
        let mut beat = Heartbeat::new(t0);
        beat.seen(t0 + Duration::from_secs(25));
        assert!(!beat.dead(t0 + Duration::from_secs(50)));
        assert!(beat.dead(t0 + Duration::from_secs(56)));
    }
}
