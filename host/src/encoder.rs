//! Détection de l'encodeur H.264 : VA (Intel/AMD) > NVENC > x264 (repli logiciel).
use gstreamer as gst;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoder {
    Va,
    VaapiLegacy,
    Nvenc,
    X264,
}

impl Encoder {
    /// Fragment de pipeline : Constrained Baseline, GOP court, pas de B-frames.
    pub fn pipeline_fragment(self, bitrate_kbps: u32) -> String {
        match self {
            Encoder::Va => format!("vah264enc bitrate={bitrate_kbps} key-int-max=30 b-frames=0 ! video/x-h264,profile=constrained-baseline"),
            // cabac et dct8x8 coupés : Constrained Baseline, seul profil H.264 que tous les téléphones décodent en WebRTC.
            Encoder::VaapiLegacy => format!("vaapih264enc bitrate={bitrate_kbps} keyframe-period=30 max-bframes=0 cabac=false dct8x8=false ! video/x-h264,profile=constrained-baseline"),
            Encoder::Nvenc => format!("nvh264enc bitrate={bitrate_kbps} gop-size=30 bframes=0 zerolatency=true ! video/x-h264,profile=constrained-baseline"),
            Encoder::X264 => format!("x264enc tune=zerolatency speed-preset=ultrafast bitrate={bitrate_kbps} key-int-max=30 bframes=0 cabac=false byte-stream=true ! video/x-h264,profile=constrained-baseline"),
        }
    }

    /// Format brut attendu par l'encodeur, après videoconvert.
    pub fn raw_format(self) -> &'static str {
        match self {
            Encoder::X264 => "I420",
            Encoder::Va | Encoder::VaapiLegacy | Encoder::Nvenc => "NV12",
        }
    }
}

/// Premier élément disponible dans l'ordre de priorité.
/// Le driver VA-API par défaut est le bon : forcer i965 sur ce GPU fait planter l'encodeur.
pub fn detect() -> Encoder {
    let has = |n: &str| gst::ElementFactory::find(n).is_some();
    if has("vah264enc") { Encoder::Va }
    else if has("vaapih264enc") { Encoder::VaapiLegacy }
    else if has("nvh264enc") { Encoder::Nvenc }
    else { Encoder::X264 }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn x264_is_zerolatency_baseline() {
        let s = Encoder::X264.pipeline_fragment(2500);
        assert!(s.contains("tune=zerolatency"));
        assert!(s.contains("bframes=0"));
        assert!(s.contains("cabac=false"));
        assert!(s.contains("constrained-baseline"));
        assert_eq!(Encoder::X264.raw_format(), "I420");
        assert_eq!(Encoder::VaapiLegacy.raw_format(), "NV12");
    }
}
