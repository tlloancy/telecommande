//! Lettres du téléphone vers le clavier X11.
//!
//! xkbcommon donne le keycode de la disposition réelle (AZERTY : « a » n’est pas
//! la touche américaine). Les niveaux d’une touche pc105 sont, dans l’ordre :
//! rien, Maj, AltGr, Maj+AltGr. XTest envoie ces keycodes à la session.

use anyhow::{anyhow, Context, Result};
use std::collections::HashMap;
use std::sync::Mutex;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{ConnectionExt, KEY_PRESS_EVENT, KEY_RELEASE_EVENT};
use x11rb::protocol::xtest;
use x11rb::rust_connection::RustConnection;
use xkbcommon::xkb::{self, Keycode, Keymap, Keysym};

const SHIFT_L: u32 = 0xffe1;
const LEVEL3: u32 = 0xfe03;

#[derive(Clone, Debug, PartialEq, Eq)]
struct Chord {
    mods: Vec<u8>,
    key: u8,
}

struct Layout {
    chords: HashMap<char, Chord>,
    dead: HashMap<u32, Chord>,
}

static LAYOUT: Mutex<Option<Layout>> = Mutex::new(None);

/// Connexion X11 gardée entre les frappes : s’en reconnecter une à chaque message coûte
/// un aller-retour (connexion + QueryExtension XTest) par lettre, et les touches
/// appuyées / relâchées partaient sur des connexions différentes.
static CONN: Mutex<Option<RustConnection>> = Mutex::new(None);

fn with_conn<T>(f: impl Fn(&RustConnection) -> Result<T>) -> Result<T> {
    let mut slot = CONN.lock().unwrap_or_else(|e| e.into_inner());
    let mut last = None;
    for _ in 0..2 {
        if slot.is_none() {
            let (conn, _) = RustConnection::connect(None).context("connexion X11")?;
            *slot = Some(conn);
        }
        let Some(conn) = slot.as_ref() else { continue };
        match f(conn) {
            Ok(v) => return Ok(v),
            Err(err) => {
                // Connexion probablement morte (session relancée) : on repart de zéro une fois.
                tracing::debug!("X11 : reconnexion ({err})");
                *slot = None;
                last = Some(err);
            }
        }
    }
    Err(last.unwrap_or_else(|| anyhow!("connexion X11 indisponible")))
}

/// Caractères que Gboard insère mais que la disposition n’a pas : équivalent tapable.
fn fallback(ch: char) -> &'static str {
    match ch {
        '\u{2018}' | '\u{2019}' | '\u{02bc}' => "'",
        '\u{201c}' | '\u{201d}' | '\u{00ab}' | '\u{00bb}' => "\"",
        '\u{2026}' => "...",
        '\u{00a0}' | '\u{202f}' | '\u{2009}' | '\u{200b}' => " ",
        '\u{2010}' | '\u{2011}' | '\u{2013}' | '\u{2014}' => "-",
        _ => "",
    }
}

pub fn type_text(text: &str) -> Result<()> {
    with_conn(|conn| {
        for ch in text.chars() {
            if ch == '\n' || ch == '\r' {
                tap(conn, evdev_to_x(28))?;
                continue;
            }
            let mut seq = sequence_for(ch)?;
            if seq.is_empty() {
                for alt in fallback(ch).chars() {
                    seq.extend(sequence_for(alt)?);
                }
            }
            if seq.is_empty() {
                // warn (et non debug) : un caractère perdu doit se voir dans les logs.
                tracing::warn!("aucune touche pour {ch:?} (U+{:04X}), ignoré", u32::from(ch));
                continue;
            }
            for chord in seq {
                play(conn, &chord)?;
            }
        }
        conn.flush().context("flush")?;
        Ok(())
    })
}

/// Pose le pointeur au pixel du bureau. Même connexion que la frappe.
pub fn warp_pointer(x: i32, y: i32) -> Result<()> {
    with_conn(|conn| {
        let root = conn.setup().roots.first().context("écran X11")?.root;
        let x = i16::try_from(x).unwrap_or(if x < 0 { i16::MIN } else { i16::MAX });
        let y = i16::try_from(y).unwrap_or(if y < 0 { i16::MIN } else { i16::MAX });
        conn.warp_pointer(0u32, root, 0, 0, 0, 0, x, y).context("warp")?;
        // Aller-retour : le clic qui suit passe par uinput, un autre chemin.
        // Sans ça, le clic peut partir avant que le serveur ait bougé le pointeur.
        conn.get_input_focus().context("sync")?.reply().context("sync")?;
        Ok(())
    })
}

/// `code` est un keycode evdev (Retour arrière = 14, Entrée = 28).
pub fn stroke(code: u16, down: bool) -> Result<()> {
    with_conn(|conn| {
        stroke_x(conn, evdev_to_x(code), down)?;
        conn.flush().context("flush")?;
        Ok(())
    })
}

fn evdev_to_x(code: u16) -> u8 {
    u8::try_from(code.saturating_add(8)).unwrap_or(0)
}

fn stroke_x(conn: &RustConnection, key: u8, down: bool) -> Result<()> {
    if key == 0 {
        return Ok(());
    }
    let kind = if down { KEY_PRESS_EVENT } else { KEY_RELEASE_EVENT };
    xtest::fake_input(conn, kind, key, 0, 0, 0, 0, 0).context("xtest")?;
    Ok(())
}

fn tap(conn: &RustConnection, key: u8) -> Result<()> {
    stroke_x(conn, key, true)?;
    stroke_x(conn, key, false)
}

fn play(conn: &RustConnection, chord: &Chord) -> Result<()> {
    for m in &chord.mods {
        stroke_x(conn, *m, true)?;
    }
    stroke_x(conn, chord.key, true)?;
    stroke_x(conn, chord.key, false)?;
    for m in chord.mods.iter().rev() {
        stroke_x(conn, *m, false)?;
    }
    Ok(())
}

fn sequence_for(ch: char) -> Result<Vec<Chord>> {
    let mut slot = LAYOUT.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        let (rules, model, layout_name, variant) = rmlvo();
        let built = compile(&rules, &model, &layout_name, &variant)
            .with_context(|| format!("disposition {layout_name} ({variant})"))?;
        tracing::info!("clavier : {layout_name} {variant}");
        *slot = Some(built);
    }
    Ok(slot.as_ref().map(|layout| layout.sequence(ch)).unwrap_or_default())
}

impl Layout {
    /// Une lettre directe, ou une touche morte puis la lettre (â = ^ + a).
    fn sequence(&self, ch: char) -> Vec<Chord> {
        if let Some(chord) = self.chords.get(&ch) {
            return vec![chord.clone()];
        }
        let Some((dead, base)) = composed(ch) else { return Vec::new() };
        match (self.dead.get(&dead), self.chords.get(&base)) {
            (Some(dead_chord), Some(base_chord)) => vec![dead_chord.clone(), base_chord.clone()],
            _ => Vec::new(),
        }
    }
}

fn composed(ch: char) -> Option<(u32, char)> {
    Some(match ch {
        'â' => (0xfe52, 'a'),
        'Â' => (0xfe52, 'A'),
        'ê' => (0xfe52, 'e'),
        'Ê' => (0xfe52, 'E'),
        'î' => (0xfe52, 'i'),
        'Î' => (0xfe52, 'I'),
        'ô' => (0xfe52, 'o'),
        'Ô' => (0xfe52, 'O'),
        'û' => (0xfe52, 'u'),
        'Û' => (0xfe52, 'U'),
        'ë' => (0xfe57, 'e'),
        'Ë' => (0xfe57, 'E'),
        'ï' => (0xfe57, 'i'),
        'Ï' => (0xfe57, 'I'),
        'ü' => (0xfe57, 'u'),
        'Ü' => (0xfe57, 'U'),
        'ÿ' => (0xfe57, 'y'),
        _ => return None,
    })
}

fn rmlvo() -> (String, String, String, String) {
    let mut rules = "evdev".to_string();
    let mut model = "pc105".to_string();
    let mut layout = "fr".to_string();
    let mut variant = String::new();
    let Ok(out) = std::process::Command::new("setxkbmap").arg("-query").output() else {
        return (rules, model, layout, variant);
    };
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let Some((key, value)) = line.split_once(':') else { continue };
        let first = value.split(',').next().unwrap_or(value).trim();
        if first.is_empty() {
            continue;
        }
        match key.trim() {
            "rules" => rules = first.to_string(),
            "model" => model = first.to_string(),
            "layout" => layout = first.to_string(),
            "variant" => variant = first.to_string(),
            _ => {}
        }
    }
    (rules, model, layout, variant)
}

fn compile(rules: &str, model: &str, layout: &str, variant: &str) -> Result<Layout> {
    let context = xkb::Context::new(xkb::CONTEXT_NO_FLAGS);
    let keymap = Keymap::new_from_names(&context, rules, model, layout, variant, None, 0)
        .ok_or_else(|| anyhow!("keymap introuvable"))?;
    Ok(index_keymap(&keymap))
}

fn index_keymap(keymap: &Keymap) -> Layout {
    let shift = find_key(keymap, SHIFT_L).unwrap_or(0);
    let level3 = find_key(keymap, LEVEL3).unwrap_or(0);
    let mut chords = HashMap::new();
    let mut dead = HashMap::new();
    let min = keymap.min_keycode().raw();
    let max = keymap.max_keycode().raw();
    for level in 0..4 {
        for code in min..=max {
            let key = Keycode::new(code);
            if keymap.num_layouts_for_key(key) == 0 || keymap.num_levels_for_key(key, 0) <= level {
                continue;
            }
            let syms = keymap.key_get_syms_by_level(key, 0, level);
            if syms.len() != 1 {
                continue;
            }
            let Ok(key_u8) = u8::try_from(code) else { continue };
            let chord = Chord { mods: mods_for_level(level, shift, level3), key: key_u8 };
            let raw = syms[0].raw();
            if (0xfe50..=0xfe6f).contains(&raw) {
                dead.entry(raw).or_insert(chord);
                continue;
            }
            let Some(ch) = char::from_u32(xkb::keysym_to_utf32(syms[0])) else { continue };
            if ch.is_control() {
                continue;
            }
            chords.entry(ch).or_insert(chord);
        }
    }
    Layout { chords, dead }
}

fn mods_for_level(level: u32, shift: u8, level3: u8) -> Vec<u8> {
    match level {
        1 if shift != 0 => vec![shift],
        2 if level3 != 0 => vec![level3],
        3 if shift != 0 && level3 != 0 => vec![shift, level3],
        _ => Vec::new(),
    }
}

fn find_key(keymap: &Keymap, sym: u32) -> Option<u8> {
    let want = Keysym::new(sym);
    let min = keymap.min_keycode().raw();
    let max = keymap.max_keycode().raw();
    for code in min..=max {
        let key = Keycode::new(code);
        if keymap.num_layouts_for_key(key) == 0 || keymap.num_levels_for_key(key, 0) == 0 {
            continue;
        }
        if keymap.key_get_syms_by_level(key, 0, 0).contains(&want) {
            return u8::try_from(code).ok();
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn french() -> Layout {
        compile("evdev", "pc105", "fr", "latin9").unwrap()
    }

    #[test]
    fn azerty_letters_use_the_french_keys() {
        let layout = french();
        let a = layout.chords.get(&'a').expect("a");
        let q = layout.chords.get(&'q').expect("q");
        let upper = layout.chords.get(&'A').expect("A");
        assert_ne!(a.key, q.key);
        assert!(a.mods.is_empty());
        assert_eq!(upper.key, a.key);
        assert_eq!(upper.mods.len(), 1);
        assert!(layout.chords.get(&'é').is_some());
        let hat = layout.sequence('â');
        assert!(hat.len() == 1 || hat.last().is_some_and(|c| c.key == a.key));
    }

    #[test]
    fn typographic_characters_fall_back_to_typable_ones() {
        assert_eq!(fallback('\u{2019}'), "'");
        assert_eq!(fallback('\u{2026}'), "...");
        assert_eq!(fallback('\u{202f}'), " ");
        assert_eq!(fallback('a'), "");
    }
}
