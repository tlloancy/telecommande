//! Géométrie des écrans X11, lue via `xrandr --listmonitors`.
//! Choisir un écran recadre la vue sur le téléphone et place le curseur au centre
//! de cet écran, pour que la souris relative parte de là où l'on regarde.

use anyhow::{anyhow, Context, Result};

#[derive(Debug, Clone, PartialEq)]
pub struct Screen {
    pub output: String,
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub w: u32,
    pub h: u32,
    pub primary: bool,
}

pub fn list() -> Result<Vec<Screen>> {
    let out = std::process::Command::new("xrandr")
        .arg("--listmonitors")
        .output()
        .context("xrandr")?;
    if !out.status.success() {
        anyhow::bail!("xrandr a échoué");
    }
    parse_listmonitors(&String::from_utf8_lossy(&out.stdout))
}

pub fn layout_message() -> Result<String> {
    let screens = list()?;
    if screens.is_empty() {
        anyhow::bail!("aucun écran");
    }
    let w = screens.iter().map(|s| s.x + s.w as i32).max().unwrap_or(0);
    let h = screens.iter().map(|s| s.y + s.h as i32).max().unwrap_or(0);
    Ok(serde_json::json!({
        "t": "layout",
        "w": w,
        "h": h,
        "screens": screens.iter().map(|s| serde_json::json!({
            "name": s.output,
            "label": s.label,
            "x": s.x,
            "y": s.y,
            "w": s.w,
            "h": s.h,
        })).collect::<Vec<_>>(),
    })
    .to_string())
}

/// Amène le pointeur au centre de la sortie demandée.
pub fn focus(output: &str) -> Result<()> {
    let screens = list()?;
    let screen = screens
        .iter()
        .find(|s| s.output == output)
        .ok_or_else(|| anyhow!("écran inconnu"))?;
    crate::keyboard::warp_pointer(screen.x + screen.w as i32 / 2, screen.y + screen.h as i32 / 2)
}

pub fn parse_listmonitors(text: &str) -> Result<Vec<Screen>> {
    let mut screens = Vec::new();
    for line in text.lines() {
        let Some(screen) = parse_line(line) else { continue };
        screens.push(screen);
    }
    if screens.is_empty() {
        anyhow::bail!("xrandr n'a listé aucun écran");
    }
    let ext = screens.iter().filter(|s| !is_internal(&s.output)).count();
    for screen in &mut screens {
        screen.label = if is_internal(&screen.output) {
            "Interne".into()
        } else if ext == 1 {
            "Externe".into()
        } else {
            screen.output.clone()
        };
    }
    Ok(screens)
}

fn parse_line(line: &str) -> Option<Screen> {
    let rest = line.trim().split_once(':')?.1.trim();
    let mut parts = rest.split_whitespace();
    let head = parts.next()?;
    let geom = parts.next()?;
    if !head.starts_with('+') {
        return None;
    }
    let primary = head.contains('*');
    let output = head.trim_start_matches(['+', '*']).to_string();
    if output.is_empty() {
        return None;
    }
    let (w, h, x, y) = parse_geom(geom)?;
    Some(Screen { output, label: String::new(), x, y, w, h, primary })
}

/// `1440/252x960/168+0+0` ou `1920/500x1080/280+-1920+0`.
fn parse_geom(geom: &str) -> Option<(u32, u32, i32, i32)> {
    let x_at = geom.find('x')?;
    let w: u32 = geom[..x_at].split('/').next()?.parse().ok()?;
    let rest = &geom[x_at + 1..];
    let sep = rest.find(['+', '-'])?;
    let h: u32 = rest[..sep].split('/').next()?.parse().ok()?;
    let (x, y) = parse_pos(&rest[sep..])?;
    Some((w, h, x, y))
}

fn parse_pos(pos: &str) -> Option<(i32, i32)> {
    let body = pos.strip_prefix('+').unwrap_or(pos);
    let y_sep = body[1..].find('+')? + 1;
    let x: i32 = body[..y_sep].parse().ok()?;
    let y: i32 = body[y_sep + 1..].parse().ok()?;
    Some((x, y))
}

fn is_internal(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    n.starts_with("EDP") || n.starts_with("LVDS") || n.starts_with("DSI")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_screens_get_short_labels_and_geometry() {
        let text = "\
Monitors: 2
 0: +*eDP-1 1440/252x960/168+0+0  eDP-1
 1: +DP-2 1920/1920x1080/1080+1440+0  DP-2
";
        let screens = parse_listmonitors(text).unwrap();
        assert_eq!(screens[0].label, "Interne");
        assert!(screens[0].primary);
        assert_eq!((screens[0].w, screens[0].h, screens[0].x), (1440, 960, 0));
        assert_eq!(screens[1].label, "Externe");
        assert_eq!((screens[1].x, screens[1].w, screens[1].h), (1440, 1920, 1080));
    }

    #[test]
    fn negative_origin_stays_intact() {
        let text = " 0: +HDMI-1 1920/500x1080/280+-1920+0  HDMI-1\n";
        let screens = parse_listmonitors(text).unwrap();
        assert_eq!((screens[0].x, screens[0].y, screens[0].w), (-1920, 0, 1920));
    }
}
