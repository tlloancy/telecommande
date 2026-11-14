//! Démon hôte : sert la PWA, gère l'appairage, héberge la signalisation.
mod capture;
mod clipboard;
mod encoder;
mod input;
mod keyboard;
mod pairing;
mod screens;
mod server;

use capture::CaptureOpts;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser)]
#[command(version, about = "Télécommande de bureau Linux")]
struct Cli {
    /// Port HTTPS
    #[arg(long, default_value_t = 8443)]
    port: u16,
    /// Dossier de la PWA compilée
    #[arg(long, default_value = "web/dist")]
    web_dir: PathBuf,
    /// Affiche un QR à jeton permanent (sans expiration) et quitte. Le service en cours le reconnaît.
    #[arg(long)]
    pair: bool,
    /// Durée de vie du QR d'appairage, en secondes (30 à 3600). Il se renouvelle seul à l'expiration.
    #[arg(long, env = "TELECOMMANDE_PAIR_TTL", default_value_t = 900,
          value_parser = clap::value_parser!(u64).range(30..=3600))]
    pair_ttl: u64,
    /// Écoute aussi hors du LAN (déconseillé). Par défaut : adresses privées, loopback et Tailscale.
    #[arg(long)]
    listen_all: bool,
    /// Images par seconde envoyées au téléphone.
    #[arg(long, default_value_t = 30)]
    fps: u32,
    /// Débit H.264 en kbit/s.
    #[arg(long, default_value_t = 4000)]
    bitrate: u32,
    /// Largeur max de l'image encodée. 0 garde la taille du bureau.
    #[arg(long, default_value_t = 0)]
    max_width: u32,
    /// Vérifie qu'une image de l'écran traverse WebRTC, sans téléphone.
    #[arg(long)]
    self_test: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    gstreamer::init()?;
    capture::ensure_glib_loop();
    let cli = Cli::parse();
    let opts = CaptureOpts {
        fps: cli.fps,
        max_width: cli.max_width,
        bitrate_kbps: cli.bitrate,
    };
    let enc = encoder::detect();
    tracing::info!("encodeur retenu : {enc:?}");
    if cli.self_test {
        capture::self_test(enc, opts)?;
        println!("self-test : des images de l'écran sont arrivées par WebRTC ({enc:?}).");
        return Ok(());
    }

    let cfg_dir = pairing::config_dir()?;
    let tls = pairing::ensure_tls(&cfg_dir)?;
    let store = pairing::TokenStore::load(&cfg_dir)?;

    if cli.pair {
        let token = store.create_token()?;
        pairing::print_qr(cli.port, &format!("t={token}"), None)?;
        return Ok(());
    }

    // Plus de jeton permanent créé (et imprimé dans le journal) à chaque démarrage :
    // le QR porte un code court, à usage unique, échangé contre un jeton par POST /pair.
    let pairing = pairing::Pairing::new(std::time::Duration::from_secs(cli.pair_ttl));
    tokio::spawn(pairing::announce_loop(pairing.clone(), cli.port));

    server::run(server::Config {
        pairing,
        port: cli.port,
        web_dir: cli.web_dir,
        tls,
        store,
        encoder: enc,
        capture: opts,
        listen_all: cli.listen_all,
    })
    .await
}
