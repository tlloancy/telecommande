//! Capture d'écran X11 (GetImage), réduite si besoin, encodée en PNG.
use anyhow::{anyhow, bail, Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, ImageFormat};
use x11rb::rust_connection::RustConnection;

pub struct Shot {
    pub png: Vec<u8>,
    /// Taille de l'image renvoyée.
    pub w: u32,
    pub h: u32,
    /// Région du bureau capturée (pixels réels).
    pub x: i32,
    pub y: i32,
    pub region_w: u32,
    pub region_h: u32,
    /// image = bureau * scale.
    pub scale: f64,
}

pub fn desktop_size() -> Result<(u32, u32)> {
    let (conn, n) = RustConnection::connect(None).context("connexion X11")?;
    let s = &conn.setup().roots[n];
    Ok((u32::from(s.width_in_pixels), u32::from(s.height_in_pixels)))
}

pub fn pointer() -> Result<(i32, i32)> {
    let (conn, n) = RustConnection::connect(None).context("connexion X11")?;
    let root = conn.setup().roots[n].root;
    let r = conn.query_pointer(root)?.reply()?;
    Ok((i32::from(r.root_x), i32::from(r.root_y)))
}

/// Capture la région (x, y, w, h) du bureau, réduite pour que son grand côté <= `max_dim`.
pub fn capture(x: i32, y: i32, w: u32, h: u32, max_dim: u32) -> Result<Shot> {
    let (conn, n) = RustConnection::connect(None).context("connexion X11")?;
    let setup = conn.setup();
    let screen = &setup.roots[n];
    let (dw, dh) = (i32::from(screen.width_in_pixels), i32::from(screen.height_in_pixels));
    let x0 = x.clamp(0, dw);
    let y0 = y.clamp(0, dh);
    let w = (w as i32).min(dw - x0).max(0) as u32;
    let h = (h as i32).min(dh - y0).max(0) as u32;
    if w == 0 || h == 0 {
        bail!("région vide");
    }
    let bpp = setup
        .pixmap_formats
        .iter()
        .find(|f| f.depth == screen.root_depth)
        .map(|f| f.bits_per_pixel)
        .ok_or_else(|| anyhow!("format de pixel inconnu"))?;
    if bpp != 32 || setup.image_byte_order != x11rb::protocol::xproto::ImageOrder::LSB_FIRST {
        bail!("format de pixel non géré ({bpp} bits) : seul le BGRX 32 bits est géré");
    }
    let reply = conn
        .get_image(ImageFormat::Z_PIXMAP, screen.root, x0 as i16, y0 as i16, w as u16, h as u16, !0)?
        .reply()
        .context("GetImage")?;
    let src = reply.data;
    if src.len() < (w * h * 4) as usize {
        bail!("image tronquée");
    }
    let scale = (f64::from(max_dim) / f64::from(w.max(h))).min(1.0);
    let (ow, oh) = (((f64::from(w) * scale).round() as u32).max(1), ((f64::from(h) * scale).round() as u32).max(1));
    let rgb = downscale(&src, w, h, ow, oh);

    let mut png = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut png, ow, oh);
        enc.set_color(png::ColorType::Rgb);
        enc.set_depth(png::BitDepth::Eight);
        enc.set_compression(png::Compression::Fast);
        let mut writer = enc.write_header().context("png")?;
        writer.write_image_data(&rgb).context("png")?;
    }
    Ok(Shot { png, w: ow, h: oh, x: x0, y: y0, region_w: w, region_h: h, scale: f64::from(ow) / f64::from(w) })
}

/// BGRX (w x h) -> RGB (ow x oh), moyenne par zone.
fn downscale(src: &[u8], w: u32, h: u32, ow: u32, oh: u32) -> Vec<u8> {
    let mut out = Vec::with_capacity((ow * oh * 3) as usize);
    for oy in 0..oh {
        let sy0 = (u64::from(oy) * u64::from(h) / u64::from(oh)) as u32;
        let sy1 = ((u64::from(oy + 1) * u64::from(h)).div_ceil(u64::from(oh)) as u32).clamp(sy0 + 1, h);
        for ox in 0..ow {
            let sx0 = (u64::from(ox) * u64::from(w) / u64::from(ow)) as u32;
            let sx1 = ((u64::from(ox + 1) * u64::from(w)).div_ceil(u64::from(ow)) as u32).clamp(sx0 + 1, w);
            let (mut r, mut g, mut b, mut n) = (0u32, 0u32, 0u32, 0u32);
            for sy in sy0..sy1 {
                let row = (sy * w) as usize * 4;
                for sx in sx0..sx1 {
                    let i = row + sx as usize * 4;
                    b += u32::from(src[i]);
                    g += u32::from(src[i + 1]);
                    r += u32::from(src[i + 2]);
                    n += 1;
                }
            }
            out.extend_from_slice(&[(r / n) as u8, (g / n) as u8, (b / n) as u8]);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn downscale_averages_blocks_and_swaps_to_rgb() {
        // 2x2 : rouge, rouge / bleu, bleu (en BGRX) -> 1x1 violet.
        let red = [0, 0, 255, 0];
        let blue = [255, 0, 0, 0];
        let src: Vec<u8> = [red, red, blue, blue].concat();
        assert_eq!(downscale(&src, 2, 2, 1, 1), vec![127, 0, 127]);
        // Sans réduction : BGRX -> RGB.
        assert_eq!(downscale(&red, 1, 1, 1, 1), vec![255, 0, 0]);
    }
}
