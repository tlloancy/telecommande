//! Démon hôte : sert la PWA, gère l'appairage, héberge la signalisation.
mod capture;
mod clipboard;
mod encoder;
mod input;
mod pairing;
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
    /// Affiche un nouveau QR d'appairage et quitte
    #[arg(long)]
    pair: bool,
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

    let token = store.create_token()?;
    let urls = pairing::page_urls(cli.port, &token);
    let url = urls.first().cloned().unwrap_or_default();
    println!("\nScanne ce QR avec le téléphone (même Wi-Fi) :\n{}\n{url}\n", pairing::qr_ascii(&url)?);
    for extra in urls.iter().skip(1) {
        println!("Autre adresse : {extra}");
    }
    if cli.pair {
        return Ok(());
    }

    server::run(server::Config {
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
