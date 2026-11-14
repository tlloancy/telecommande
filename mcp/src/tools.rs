//! Les outils exposés à l'agent et leur exécution.
use crate::{keyboard, keys, safety, screens, shot};
use base64::Engine;
use serde_json::{json, Value};
use std::time::Duration;

pub const MAX_TEXT: usize = 5000;

pub fn definitions() -> Value {
    let xy = |what: &str| json!({
        "type": "object",
        "properties": {
            "x": {"type": "integer", "description": format!("X {what}, en pixels de la dernière capture")},
            "y": {"type": "integer", "description": format!("Y {what}, en pixels de la dernière capture")}
        },
        "required": ["x", "y"]
    });
    json!([
        {
            "name": "list_screens",
            "description": "Liste les écrans (nom, position et taille en pixels du bureau).",
            "inputSchema": {"type": "object", "properties": {}}
        },
        {
            "name": "screenshot",
            "description": "Capture le bureau (ou un seul écran) en PNG. Les coordonnées des autres outils sont celles de la DERNIÈRE capture : refais une capture après un changement d'écran.",
            "inputSchema": {"type": "object", "properties": {
                "screen": {"type": "string", "description": "Nom d'écran de list_screens ; tout le bureau si absent"},
                "max_dim": {"type": "integer", "description": "Plus grand côté de l'image en pixels (256 à 4096, 1568 par défaut)"}
            }}
        },
        {
            "name": "mouse_move",
            "description": "Déplace le pointeur.",
            "inputSchema": xy("de destination")
        },
        {
            "name": "click",
            "description": "Déplace le pointeur puis clique.",
            "inputSchema": {"type": "object", "properties": {
                "x": {"type": "integer"}, "y": {"type": "integer"},
                "button": {"type": "string", "enum": ["left", "right", "middle"], "description": "left par défaut"},
                "double": {"type": "boolean", "description": "double-clic"}
            }, "required": ["x", "y"]}
        },
        {
            "name": "drag",
            "description": "Glisser-déposer avec le bouton gauche.",
            "inputSchema": {"type": "object", "properties": {
                "from_x": {"type": "integer"}, "from_y": {"type": "integer"},
                "to_x": {"type": "integer"}, "to_y": {"type": "integer"}
            }, "required": ["from_x", "from_y", "to_x", "to_y"]}
        },
        {
            "name": "scroll",
            "description": "Fait défiler en crans de molette, à la position donnée ou sous le pointeur. dy > 0 vers le bas, dx > 0 vers la droite.",
            "inputSchema": {"type": "object", "properties": {
                "x": {"type": "integer"}, "y": {"type": "integer"},
                "dx": {"type": "integer"}, "dy": {"type": "integer"}
            }}
        },
        {
            "name": "type_text",
            "description": "Tape du texte avec la disposition réelle du clavier (accents compris). Pour Entrée, Tab ou un raccourci, utilise key.",
            "inputSchema": {"type": "object", "properties": {"text": {"type": "string"}}, "required": ["text"]}
        },
        {
            "name": "key",
            "description": "Appuie sur une touche ou un raccourci : « Enter », « Escape », « F5 », « ctrl+c », « alt+F4 », « ctrl+shift+t ». Lettres en minuscules.",
            "inputSchema": {"type": "object", "properties": {"key": {"type": "string"}}, "required": ["key"]}
        }
    ])
}

pub struct Tools {
    pub safety: safety::Safety,
    /// Dernière capture : (x, y, échelle) du bureau réel vers l'image.
    last: Option<(i32, i32, f64)>,
}

fn int(args: &Value, key: &str) -> Result<i32, String> {
    args.get(key)
        .and_then(Value::as_i64)
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| format!("paramètre entier « {key} » manquant ou invalide"))
}

fn opt_int(args: &Value, key: &str) -> i32 {
    args.get(key).and_then(Value::as_i64).and_then(|v| i32::try_from(v).ok()).unwrap_or(0)
}

fn text(s: impl Into<String>) -> Value {
    json!({"type": "text", "text": s.into()})
}

impl Tools {
    pub fn new(safety: safety::Safety) -> Self {
        Self { safety, last: None }
    }

    /// Image -> bureau.
    fn to_desktop(&self, x: i32, y: i32) -> (i32, i32) {
        let (ox, oy, s) = self.last.unwrap_or((0, 0, 1.0));
        ((f64::from(ox) + f64::from(x) / s).round() as i32, (f64::from(oy) + f64::from(y) / s).round() as i32)
    }

    pub fn call(&mut self, name: &str, args: &Value) -> Result<Vec<Value>, String> {
        if name == "list_screens" {
            return self.list_screens();
        }
        if !matches!(name, "screenshot" | "mouse_move" | "click" | "drag" | "scroll" | "type_text" | "key") {
            return Err(format!("outil inconnu : {name}"));
        }
        self.safety.authorize().inspect_err(|why| safety::journal(name, "{}", &format!("refusé : {why}")))?;
        let detail = redacted(name, args);
        let result = match name {
            "screenshot" => self.screenshot(args),
            "mouse_move" => self.mouse_move(args),
            "click" => self.click(args),
            "drag" => self.drag(args),
            "scroll" => self.scroll(args),
            "type_text" => self.type_text(args),
            _ => self.key(args),
        };
        match &result {
            Ok(_) => safety::journal(name, &detail, "ok"),
            Err(e) => safety::journal(name, &detail, &format!("erreur : {e}")),
        }
        result
    }

    fn list_screens(&self) -> Result<Vec<Value>, String> {
        let list = screens::list().map_err(|e| e.to_string())?;
        let rows: Vec<Value> = list
            .iter()
            .map(|s| json!({"name": s.output, "x": s.x, "y": s.y, "width": s.w, "height": s.h, "primary": s.primary}))
            .collect();
        Ok(vec![text(serde_json::to_string_pretty(&rows).unwrap_or_default())])
    }

    fn screenshot(&mut self, args: &Value) -> Result<Vec<Value>, String> {
        let max_dim = args.get("max_dim").and_then(Value::as_u64).unwrap_or(1568).clamp(256, 4096) as u32;
        let (x, y, w, h) = match args.get("screen").and_then(Value::as_str) {
            Some(name) => {
                let list = screens::list().map_err(|e| e.to_string())?;
                let s = list.iter().find(|s| s.output == name).ok_or_else(|| format!("écran inconnu : {name}"))?;
                (s.x, s.y, s.w, s.h)
            }
            None => {
                let (w, h) = shot::desktop_size().map_err(|e| e.to_string())?;
                (0, 0, w, h)
            }
        };
        let shot = shot::capture(x, y, w, h, max_dim).map_err(|e| format!("{e:#}"))?;
        self.last = Some((shot.x, shot.y, shot.scale));
        let ptr = shot::pointer().ok().map(|(px, py)| {
            let (ix, iy) = (
                ((f64::from(px - shot.x)) * shot.scale).round() as i32,
                ((f64::from(py - shot.y)) * shot.scale).round() as i32,
            );
            format!(" Pointeur à ({ix}, {iy}) dans l'image.")
        });
        let info = format!(
            "Image {}x{} px (région du bureau {}x{} en ({}, {}), échelle {:.3}).{}",
            shot.w, shot.h, shot.region_w, shot.region_h, shot.x, shot.y, shot.scale, ptr.unwrap_or_default()
        );
        let data = base64::engine::general_purpose::STANDARD.encode(&shot.png);
        Ok(vec![text(info), json!({"type": "image", "data": data, "mimeType": "image/png"})])
    }

    fn mouse_move(&mut self, args: &Value) -> Result<Vec<Value>, String> {
        let (x, y) = self.to_desktop(int(args, "x")?, int(args, "y")?);
        keyboard::warp_pointer(x, y).map_err(|e| format!("{e:#}"))?;
        Ok(vec![text(format!("Pointeur en ({x}, {y}) sur le bureau."))])
    }

    fn click(&mut self, args: &Value) -> Result<Vec<Value>, String> {
        let (x, y) = self.to_desktop(int(args, "x")?, int(args, "y")?);
        let button = match args.get("button").and_then(Value::as_str).unwrap_or("left") {
            "left" => 1,
            "middle" => 2,
            "right" => 3,
            other => return Err(format!("bouton inconnu : {other}")),
        };
        let times = if args.get("double").and_then(Value::as_bool).unwrap_or(false) { 2 } else { 1 };
        keyboard::warp_pointer(x, y).map_err(|e| format!("{e:#}"))?;
        for i in 0..times {
            if i > 0 {
                std::thread::sleep(Duration::from_millis(60));
            }
            keyboard::mouse_button(button, true).map_err(|e| format!("{e:#}"))?;
            keyboard::mouse_button(button, false).map_err(|e| format!("{e:#}"))?;
        }
        Ok(vec![text(format!("Clic ({x}, {y}) sur le bureau."))])
    }

    fn drag(&mut self, args: &Value) -> Result<Vec<Value>, String> {
        let (x0, y0) = self.to_desktop(int(args, "from_x")?, int(args, "from_y")?);
        let (x1, y1) = self.to_desktop(int(args, "to_x")?, int(args, "to_y")?);
        keyboard::warp_pointer(x0, y0).map_err(|e| format!("{e:#}"))?;
        keyboard::mouse_button(1, true).map_err(|e| format!("{e:#}"))?;
        let mut outcome = Ok(());
        for step in 1..=10 {
            if safety::is_stopped() {
                outcome = Err("Arrêt d'urgence pendant le glisser-déposer".to_string());
                break;
            }
            let (x, y) = (x0 + (x1 - x0) * step / 10, y0 + (y1 - y0) * step / 10);
            if let Err(e) = keyboard::warp_pointer(x, y) {
                outcome = Err(format!("{e:#}"));
                break;
            }
            std::thread::sleep(Duration::from_millis(15));
        }
        // Toujours relâcher le bouton, même en cas d'arrêt d'urgence.
        let _ = keyboard::mouse_button(1, false);
        outcome?;
        Ok(vec![text(format!("Glissé de ({x0}, {y0}) à ({x1}, {y1}) sur le bureau."))])
    }

    fn scroll(&mut self, args: &Value) -> Result<Vec<Value>, String> {
        if args.get("x").is_some() && args.get("y").is_some() {
            let (x, y) = self.to_desktop(int(args, "x")?, int(args, "y")?);
            keyboard::warp_pointer(x, y).map_err(|e| format!("{e:#}"))?;
        }
        let (dx, dy) = (opt_int(args, "dx").clamp(-50, 50), opt_int(args, "dy").clamp(-50, 50));
        let notches = |n: i32, neg: u8, pos: u8| -> Result<(), String> {
            let button = if n < 0 { neg } else { pos };
            for _ in 0..n.unsigned_abs() {
                keyboard::mouse_button(button, true).map_err(|e| format!("{e:#}"))?;
                keyboard::mouse_button(button, false).map_err(|e| format!("{e:#}"))?;
            }
            Ok(())
        };
        notches(dy, 4, 5)?;
        notches(dx, 6, 7)?;
        Ok(vec![text(format!("Défilement dx={dx}, dy={dy}."))])
    }

    fn type_text(&mut self, args: &Value) -> Result<Vec<Value>, String> {
        let s = args.get("text").and_then(Value::as_str).ok_or("paramètre « text » manquant")?;
        if s.chars().count() > MAX_TEXT {
            return Err(format!("texte trop long (max {MAX_TEXT} caractères)"));
        }
        let skipped = keyboard::type_text_report(s).map_err(|e| format!("{e:#}"))?;
        let total = s.chars().count();
        if skipped.is_empty() {
            Ok(vec![text(format!("{total} caractères tapés."))])
        } else {
            let list: String = skipped.iter().collect();
            Err(format!(
                "{} caractères tapés sur {total} ; absents de la disposition du clavier et ignorés : {list}",
                total - skipped.len()
            ))
        }
    }

    fn key(&mut self, args: &Value) -> Result<Vec<Value>, String> {
        let spec = args.get("key").and_then(Value::as_str).ok_or("paramètre « key » manquant")?;
        let (mods, target) = keys::parse(spec)?;
        keyboard::shortcut(&mods, &target).map_err(|e| format!("{e:#}"))?;
        Ok(vec![text(format!("Touche {spec}."))])
    }
}

/// Arguments pour le journal : jamais le texte tapé, seulement sa longueur.
fn redacted(name: &str, args: &Value) -> String {
    if name == "type_text" {
        let n = args.get("text").and_then(Value::as_str).map_or(0, |s| s.chars().count());
        return json!({"chars": n}).to_string();
    }
    args.to_string()
}
