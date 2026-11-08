//! Capture X11 → H.264 → WebRTC.
//!
//! Le curseur n'est pas dans l'image (`show-pointer=false`) : la PWA le dessine.
//! `use-damage=false` évite les frames incomplètes d'XDamage.
//! `do-timestamp=true` : ximagesrc n'horodate pas tout seul, et webrtcbin en a besoin.
//! La queue fuit vers l'aval : si l'encodeur prend du retard, on jette l'image ancienne
//! au lieu d'empiler de la latence.
//! `latency=0` sur webrtcbin : le tampon de gigue par défaut (200 ms) dépasse le budget LAN.

use crate::{encoder::Encoder, input};
use anyhow::{anyhow, Context, Result};
use gstreamer as gst;
use gstreamer_sdp as gst_sdp;
use gstreamer_webrtc as gst_webrtc;
use gst::prelude::*;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc::UnboundedSender;

#[derive(Clone, Copy, Debug)]
pub struct CaptureOpts {
    pub fps: u32,
    pub max_width: u32,
    pub bitrate_kbps: u32,
}

/// Messages poussés vers le WebSocket de signalisation.
#[derive(Debug)]
pub enum HostSignal {
    Offer(String),
    Ice { mline: u32, candidate: String },
    Failed,
}

pub fn pipeline_description(enc: Encoder, opts: &CaptureOpts) -> String {
    let scale = if opts.max_width > 0 {
        format!(
            "videoscale ! video/x-raw,width={},pixel-aspect-ratio=1/1 ! ",
            opts.max_width
        )
    } else {
        String::new()
    };
    format!(
        "ximagesrc use-damage=false show-pointer=false do-timestamp=true ! \
         video/x-raw,framerate={fps}/1 ! videoconvert ! video/x-raw,format={fmt} ! \
         queue max-size-buffers=2 leaky=downstream ! \
         {scale}{enc} ! h264parse config-interval=-1 ! \
         rtph264pay config-interval=-1 aggregate-mode=zero-latency pt=96 ! \
         webrtcbin name=webrtc bundle-policy=max-bundle latency=0",
        fps = opts.fps.max(1),
        fmt = enc.raw_format(),
        enc = enc.pipeline_fragment(opts.bitrate_kbps),
    )
}

/// La boucle GLib par défaut doit tourner : libnice y planifie les checks ICE.
pub fn ensure_glib_loop() {
    use std::sync::Once;
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        std::thread::Builder::new()
            .name("glib-main".into())
            .spawn(|| {
                gst::glib::MainLoop::new(None, false).run();
            })
            .expect("thread GLib");
    });
}

struct WebrtcInner {
    webrtc: gst::Element,
    remote_set: bool,
    pending_ice: Vec<(u32, String)>,
}

pub struct Session {
    pipeline: gst::Pipeline,
    inner: Arc<Mutex<WebrtcInner>>,
    _channels: Vec<gst::glib::Object>,
    _watch: gst::bus::BusWatchGuard,
}

impl Session {
    /// Démarre la capture. Si l'encodeur matériel refuse le pipeline, repli x264.
    pub fn start(enc: Encoder, opts: CaptureOpts, tx: UnboundedSender<HostSignal>) -> Result<Self> {
        ensure_glib_loop();
        ensure_runtime()?;
        match start_with(enc, opts, tx.clone()) {
            Ok(session) => Ok(session),
            Err(err) if enc != Encoder::X264 => {
                tracing::warn!("encodeur {enc:?} inutilisable ({err:#}), repli x264");
                start_with(Encoder::X264, opts, tx)
            }
            Err(err) => Err(err),
        }
    }

    pub fn answer(&self, sdp: &str) -> Result<()> {
        let sdp = sdp.to_string();
        let inner = self.inner.clone();
        glib_invoke(move || apply_answer(&inner, sdp));
        Ok(())
    }

    pub fn ice(&self, mline: u32, candidate: &str) -> Result<()> {
        if candidate.is_empty() {
            return Ok(());
        }
        let candidate = candidate.to_string();
        let inner = self.inner.clone();
        glib_invoke(move || {
            let webrtc = {
                let mut guard = inner.lock().unwrap();
                if !guard.remote_set {
                    guard.pending_ice.push((mline, candidate));
                    return;
                }
                guard.webrtc.clone()
            };
            webrtc.emit_by_name::<()>("add-ice-candidate", &[&mline, &candidate]);
        });
        Ok(())
    }

    pub fn on_client_text(&self, text: &str) -> Result<()> {
        if text.len() > 256 * 1024 {
            return Ok(());
        }
        let value: serde_json::Value = match serde_json::from_str(text) {
            Ok(v) => v,
            Err(_) => return Ok(()),
        };
        match value.get("kind").and_then(|k| k.as_str()) {
            Some("answer") => {
                let sdp = value.get("sdp").and_then(|s| s.as_str()).context("sdp manquant")?;
                self.answer(sdp)
            }
            Some("ice") => {
                let candidate = value.get("candidate").and_then(|s| s.as_str()).unwrap_or("");
                let mline = value.get("sdpMLineIndex").and_then(|n| n.as_u64()).unwrap_or(0) as u32;
                self.ice(mline, candidate)
            }
            _ => Ok(()),
        }
    }

    pub fn poll_error(&self) -> Option<String> {
        poll_bus(&self.pipeline)
    }

    pub fn stop(&self) {
        let _ = self.pipeline.set_state(gst::State::Null);
        let _ = self.pipeline.state(gst::ClockTime::from_seconds(2));
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

fn ensure_runtime() -> Result<()> {
    if gst::ElementFactory::find("nicesrc").is_none() {
        anyhow::bail!(
            "gstreamer1.0-nice est absent : webrtcbin ne trouve pas nicesrc/nicesink. \
             sudo apt install gstreamer1.0-nice"
        );
    }
    if std::env::var_os("DISPLAY").is_none() {
        anyhow::bail!("DISPLAY est vide : pas de session X11 à capturer");
    }
    Ok(())
}

fn start_with(enc: Encoder, opts: CaptureOpts, tx: UnboundedSender<HostSignal>) -> Result<Session> {
    let desc = pipeline_description(enc, &opts);
    tracing::debug!("pipeline : {desc}");
    let element = gst::parse::launch(&desc).with_context(|| format!("pipeline illisible ({enc:?})"))?;
    let pipeline = element
        .downcast::<gst::Pipeline>()
        .map_err(|_| anyhow!("le pipeline n'est pas un GstPipeline"))?;
    let webrtc = pipeline
        .by_name("webrtc")
        .context("webrtcbin introuvable dans le pipeline")?;
    if webrtc.sink_pads().is_empty() {
        anyhow::bail!("webrtcbin n'a pas de pad d'entrée");
    }

    // webrtcbin refuse create-data-channel tant qu'il est NULL (is_closed).
    pipeline.set_state(gst::State::Ready).context("préparation")?;
    let (res, current, pending) = pipeline.state(gst::ClockTime::from_seconds(5));
    if res.is_err() || current < gst::State::Ready {
        let _ = pipeline.set_state(gst::State::Null);
        anyhow::bail!("pipeline resté en {current:?} avant les canaux (en attente {pending:?})");
    }

    // Canaux créés avant la négociation, sinon ils manquent à l'offre SDP.
    let move_ch = make_channel(&webrtc, "move", true)?;
    let ctrl_ch = make_channel(&webrtc, "ctrl", false)?;
    attach_input(&move_ch, false);
    attach_input(&ctrl_ch, true);

    let inner = Arc::new(Mutex::new(WebrtcInner {
        webrtc: webrtc.clone(),
        remote_set: false,
        pending_ice: Vec::new(),
    }));
    connect_signals(&webrtc, &tx);

    let tx_bus = tx.clone();
    let watch = pipeline
        .bus()
        .context("bus")?
        .add_watch(move |_, msg| {
            if let gst::MessageView::Error(err) = msg.view() {
                tracing::error!(
                    "pipeline : {} ({})",
                    err.error(),
                    err.debug().unwrap_or_default()
                );
                let _ = tx_bus.send(HostSignal::Failed);
            }
            gst::glib::ControlFlow::Continue
        })
        .context("bus watch")?;

    pipeline
        .set_state(gst::State::Playing)
        .context("passage en lecture")?;
    let (res, current, pending) = pipeline.state(gst::ClockTime::from_seconds(8));
    if res.is_err() || current != gst::State::Playing {
        let _ = pipeline.set_state(gst::State::Null);
        anyhow::bail!("pipeline resté en {current:?} (en attente {pending:?})");
    }

    Ok(Session {
        pipeline,
        inner,
        _channels: vec![move_ch, ctrl_ch],
        _watch: watch,
    })
}

fn make_channel(webrtc: &gst::Element, name: &str, unreliable: bool) -> Result<gst::glib::Object> {
    // max-retransmits=0 + ordered=false : DataChannel souris, pertes acceptées.
    let opts = if unreliable {
        Some(
            gst::Structure::builder("application/data-channel")
                .field("ordered", false)
                .field("max-retransmits", 0i32)
                .build(),
        )
    } else {
        None
    };
    webrtc
        .emit_by_name::<Option<gst::glib::Object>>("create-data-channel", &[&name, &opts])
        .with_context(|| format!("DataChannel {name}"))
}

fn attach_input(channel: &gst::glib::Object, reliable: bool) {
    if reliable {
        channel.connect("on-open", false, |vals| {
            send_layout(vals);
            None
        });
    }
    channel.connect("on-message-string", false, move |vals| {
        let Ok(text) = vals[1].get::<String>() else {
            return None;
        };
        if let Ok(ev) = serde_json::from_str::<input::InputEvent>(&text) {
            if reliable && matches!(ev, input::InputEvent::Hello) {
                send_layout(vals);
            }
            input::handle(ev);
        }
        None
    });
}

fn send_layout(vals: &[gst::glib::Value]) {
    let json = match crate::screens::layout_message() {
        Ok(json) => json,
        Err(err) => {
            tracing::warn!("écrans illisibles : {err}");
            return;
        }
    };
    let Some(channel) = vals.first().and_then(|v| v.get::<gst::glib::Object>().ok()) else {
        return;
    };
    channel.emit_by_name::<()>("send-string", &[&Some(json.as_str())]);
}

fn connect_signals(webrtc: &gst::Element, tx: &UnboundedSender<HostSignal>) {
    let tx_offer = tx.clone();
    let offered = Arc::new(AtomicBool::new(false));
    webrtc.connect("on-negotiation-needed", false, move |vals| {
        if offered.swap(true, Ordering::SeqCst) {
            return None;
        }
        let Ok(webrtc) = vals[0].get::<gst::Element>() else {
            return None;
        };
        let tx_offer = tx_offer.clone();
        let webrtc_set = webrtc.clone();
        let promise = gst::Promise::with_change_func(move |reply| {
            let Ok(Some(reply)) = reply else {
                tracing::error!("create-offer sans réponse");
                let _ = tx_offer.send(HostSignal::Failed);
                return;
            };
            let Ok(offer) = reply.get::<gst_webrtc::WebRTCSessionDescription>("offer") else {
                tracing::error!("offre SDP illisible");
                let _ = tx_offer.send(HostSignal::Failed);
                return;
            };
            webrtc_set.emit_by_name::<()>("set-local-description", &[&offer, &None::<gst::Promise>]);
            match offer.sdp().as_text() {
                Ok(sdp) => {
                    let _ = tx_offer.send(HostSignal::Offer(sdp));
                }
                Err(err) => tracing::error!("sdp : {err}"),
            }
        });
        webrtc.emit_by_name::<()>("create-offer", &[&None::<gst::Structure>, &promise]);
        None
    });

    let tx_ice = tx.clone();
    webrtc.connect("on-ice-candidate", false, move |vals| {
        let Ok(mline) = vals[1].get::<u32>() else {
            return None;
        };
        let Ok(candidate) = vals[2].get::<String>() else {
            return None;
        };
        if !candidate.is_empty() {
            let _ = tx_ice.send(HostSignal::Ice { mline, candidate });
        }
        None
    });
}

fn apply_answer(inner: &Arc<Mutex<WebrtcInner>>, sdp: String) {
    let msg = match gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()) {
        Ok(msg) => msg,
        Err(err) => {
            tracing::warn!("réponse SDP refusée : {err}");
            return;
        }
    };
    let desc = gst_webrtc::WebRTCSessionDescription::new(gst_webrtc::WebRTCSDPType::Answer, msg);
    let webrtc = inner.lock().unwrap().webrtc.clone();
    let inner = inner.clone();
    let promise = gst::Promise::with_change_func(move |reply| {
        if let Err(err) = reply {
            tracing::warn!("set-remote-description : {err:?}");
            return;
        }
        let pending = {
            let mut guard = inner.lock().unwrap();
            guard.remote_set = true;
            std::mem::take(&mut guard.pending_ice)
        };
        let webrtc = inner.lock().unwrap().webrtc.clone();
        for (mline, candidate) in pending {
            webrtc.emit_by_name::<()>("add-ice-candidate", &[&mline, &candidate]);
        }
    });
    webrtc.emit_by_name::<()>("set-remote-description", &[&desc, &promise]);
}

fn glib_invoke<F>(f: F)
where
    F: FnOnce() + Send + 'static,
{
    gst::glib::MainContext::default().invoke(f);
}

fn poll_bus(pipeline: &gst::Pipeline) -> Option<String> {
    let bus = pipeline.bus()?;
    let mut error = None;
    while let Some(msg) = bus.pop() {
        if let gst::MessageView::Error(err) = msg.view() {
            error = Some(format!("{}", err.error()));
        }
    }
    error
}

/// Deux webrtcbin sur la même machine : l'écran doit revenir en images décodées.
/// Synchrone : la session GStreamer n'est pas `Send`, elle reste sur ce thread.
pub fn self_test(enc: Encoder, opts: CaptureOpts) -> Result<()> {
    ensure_glib_loop();
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let recv = Recv::new()?;
    let session = Session::start(enc, opts, tx)?;
    let mut answer_set = false;
    let mut pending: Vec<(u32, String)> = Vec::new();
    let mut offer_seen = false;
    let deadline = Instant::now() + Duration::from_secs(15);

    while recv.frames() < 5 && Instant::now() < deadline {
        if let Some(err) = session.poll_error().or_else(|| recv.poll_error()) {
            anyhow::bail!("pipeline : {err}");
        }
        match rx.try_recv() {
            Ok(HostSignal::Offer(sdp)) if !answer_set => {
                if !sdp.to_ascii_lowercase().contains("h264") {
                    let _ = std::fs::write("/tmp/telecommande-offer.sdp", &sdp);
                    anyhow::bail!("l'offre ne contient pas H.264");
                }
                offer_seen = true;
                let answer = recv.accept_offer(&sdp)?;
                session.answer(&answer)?;
                answer_set = true;
                for (mline, candidate) in pending.drain(..) {
                    recv.add_ice(mline, &candidate);
                }
            }
            Ok(HostSignal::Ice { mline, candidate }) => {
                if answer_set {
                    recv.add_ice(mline, &candidate);
                } else {
                    pending.push((mline, candidate));
                }
            }
            Ok(HostSignal::Failed) => anyhow::bail!("le pipeline de capture a échoué"),
            Ok(HostSignal::Offer(_)) => {}
            Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
                std::thread::sleep(Duration::from_millis(40));
            }
            Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => break,
        }
        for (mline, candidate) in recv.drain_ice() {
            session.ice(mline, &candidate)?;
        }
    }

    let n = recv.frames();
    if n < 5 {
        anyhow::bail!(
            "pas assez d'images ({n}), offre reçue : {offer_seen}. Session X11 et gstreamer1.0-nice requis."
        );
    }
    session.stop();
    recv.stop();
    Ok(())
}

struct Recv {
    pipeline: gst::Pipeline,
    webrtc: gst::Element,
    ice_rx: std::sync::mpsc::Receiver<(u32, String)>,
    frames: Arc<AtomicU32>,
}

impl Recv {
    fn new() -> Result<Self> {
        let pipeline = gst::Pipeline::new();
        let webrtc = gst::ElementFactory::make("webrtcbin")
            .name("recv")
            .build()
            .context("webrtcbin récepteur")?;
        webrtc.set_property_from_str("bundle-policy", "max-bundle");

        let depay = gst::ElementFactory::make("rtph264depay").build()?;
        let parse = gst::ElementFactory::make("h264parse").build()?;
        let dec = gst::ElementFactory::make("avdec_h264").build()?;
        let conv = gst::ElementFactory::make("videoconvert").build()?;
        let sink = gst::ElementFactory::make("fakesink")
            .property("sync", false)
            .build()?;

        pipeline.add_many([&webrtc, &depay, &parse, &dec, &conv, &sink])?;
        gst::Element::link_many([&depay, &parse, &dec, &conv, &sink])?;

        let sinkpad = depay.static_pad("sink").context("pad décodeur")?;
        webrtc.connect("pad-added", false, move |vals| {
            let Ok(pad) = vals[1].get::<gst::Pad>() else {
                return None;
            };
            if pad.direction() != gst::PadDirection::Src || sinkpad.is_linked() {
                return None;
            }
            if let Err(err) = pad.link(&sinkpad) {
                tracing::warn!("lien récepteur : {err}");
            }
            None
        });

        let (ice_tx, ice_rx) = std::sync::mpsc::channel();
        webrtc.connect("on-ice-candidate", false, move |vals| {
            let Ok(mline) = vals[1].get::<u32>() else {
                return None;
            };
            let Ok(candidate) = vals[2].get::<String>() else {
                return None;
            };
            let _ = ice_tx.send((mline, candidate));
            None
        });

        let frames = Arc::new(AtomicU32::new(0));
        let counter = frames.clone();
        sink.static_pad("sink")
            .context("pad fakesink")?
            .add_probe(gst::PadProbeType::BUFFER, move |_, _| {
                counter.fetch_add(1, Ordering::Relaxed);
                gst::PadProbeReturn::Ok
            });

        pipeline.set_state(gst::State::Playing).context("récepteur")?;
        Ok(Self {
            pipeline,
            webrtc,
            ice_rx,
            frames,
        })
    }

    fn frames(&self) -> u32 {
        self.frames.load(Ordering::Relaxed)
    }

    fn accept_offer(&self, sdp: &str) -> Result<String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let webrtc = self.webrtc.clone();
        let sdp = sdp.to_string();
        glib_invoke(move || {
            let msg = match gst_sdp::SDPMessage::parse_buffer(sdp.as_bytes()) {
                Ok(msg) => msg,
                Err(err) => {
                    let _ = tx.send(Err(format!("offre : {err}")));
                    return;
                }
            };
            let offer = gst_webrtc::WebRTCSessionDescription::new(gst_webrtc::WebRTCSDPType::Offer, msg);
            let webrtc2 = webrtc.clone();
            let tx2 = tx.clone();
            let after_remote = gst::Promise::with_change_func(move |reply| {
                if let Err(err) = reply {
                    let _ = tx2.send(Err(format!("set-remote : {err:?}")));
                    return;
                }
                let tx3 = tx2.clone();
                let webrtc3 = webrtc2.clone();
                let created = gst::Promise::with_change_func(move |reply| {
                    let Ok(Some(reply)) = reply else {
                        let _ = tx3.send(Err("create-answer vide".into()));
                        return;
                    };
                    let Ok(answer) = reply.get::<gst_webrtc::WebRTCSessionDescription>("answer") else {
                        let _ = tx3.send(Err("answer illisible".into()));
                        return;
                    };
                    webrtc3.emit_by_name::<()>("set-local-description", &[&answer, &None::<gst::Promise>]);
                    match answer.sdp().as_text() {
                        Ok(text) => {
                            let _ = tx3.send(Ok(text));
                        }
                        Err(err) => {
                            let _ = tx3.send(Err(err.to_string()));
                        }
                    }
                });
                webrtc2.emit_by_name::<()>("create-answer", &[&None::<gst::Structure>, &created]);
            });
            webrtc.emit_by_name::<()>("set-remote-description", &[&offer, &after_remote]);
        });
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(err)) => Err(anyhow!(err)),
            Err(_) => Err(anyhow!("pas de réponse SDP du récepteur")),
        }
    }

    fn add_ice(&self, mline: u32, candidate: &str) {
        let candidate = candidate.to_string();
        let webrtc = self.webrtc.clone();
        glib_invoke(move || {
            webrtc.emit_by_name::<()>("add-ice-candidate", &[&mline, &candidate]);
        });
    }

    fn drain_ice(&self) -> Vec<(u32, String)> {
        let mut out = Vec::new();
        while let Ok(item) = self.ice_rx.try_recv() {
            out.push(item);
        }
        out
    }

    fn poll_error(&self) -> Option<String> {
        poll_bus(&self.pipeline)
    }

    fn stop(&self) {
        let _ = self.pipeline.set_state(gst::State::Null);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pipeline_hides_pointer_and_names_encoder() {
        let wide = pipeline_description(
            Encoder::X264,
            &CaptureOpts { fps: 60, max_width: 1920, bitrate_kbps: 6000 },
        );
        assert!(wide.contains("show-pointer=false"));
        assert!(wide.contains("use-damage=false"));
        assert!(wide.contains("x264enc"));
        assert!(wide.contains("videoscale"));
        assert!(wide.contains("webrtcbin"));
        assert!(wide.contains("latency=0"));
        assert!(wide.contains("leaky=downstream"));

        let native = pipeline_description(
            Encoder::VaapiLegacy,
            &CaptureOpts { fps: 30, max_width: 0, bitrate_kbps: 4000 },
        );
        assert!(!native.contains("videoscale"));
        assert!(native.contains("vaapih264enc"));
        assert!(native.contains("format=NV12"));
    }
}
