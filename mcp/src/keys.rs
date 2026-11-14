//! Noms de touches (« ctrl+shift+t », « Enter », « F5 ») vers keycodes evdev.
use crate::keyboard::Target;

const MODS: &[(&str, u16)] = &[
    ("ctrl", 29), ("control", 29), ("shift", 42), ("alt", 56), ("super", 125), ("meta", 125),
    ("win", 125),
];

const NAMED: &[(&str, u16)] = &[
    ("escape", 1), ("esc", 1), ("backspace", 14), ("tab", 15), ("enter", 28), ("return", 28),
    ("space", 57), ("home", 102), ("up", 103), ("pageup", 104), ("left", 105), ("right", 106),
    ("end", 107), ("down", 108), ("pagedown", 109), ("insert", 110), ("delete", 111),
    ("f1", 59), ("f2", 60), ("f3", 61), ("f4", 62), ("f5", 63), ("f6", 64), ("f7", 65),
    ("f8", 66), ("f9", 67), ("f10", 68), ("f11", 87), ("f12", 88),
];

/// `"ctrl+shift+t"` -> (modificateurs, touche). Une lettre ou un chiffre seul passe par la
/// disposition réelle du clavier ; les noms (Enter, F5...) par leur keycode.
pub fn parse(spec: &str) -> Result<(Vec<u16>, Target), String> {
    let mut mods = Vec::new();
    let parts: Vec<&str> = spec.split('+').map(str::trim).collect();
    let (last, held) = parts.split_last().ok_or("touche vide")?;
    if last.is_empty() {
        return Err("touche vide".into());
    }
    for m in held {
        let lower = m.to_ascii_lowercase();
        let code = MODS.iter().find(|(n, _)| *n == lower).ok_or(format!("modificateur inconnu : {m}"))?.1;
        if !mods.contains(&code) {
            mods.push(code);
        }
    }
    let lower = last.to_ascii_lowercase();
    if let Some((_, code)) = NAMED.iter().find(|(n, _)| *n == lower) {
        return Ok((mods, Target::Code(*code)));
    }
    if let Some((_, code)) = MODS.iter().find(|(n, _)| *n == lower) {
        return Ok((mods, Target::Code(*code)));
    }
    let mut chars = last.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) => Ok((mods, Target::Char(c))),
        _ => Err(format!("touche inconnue : {last}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combo_is_split_into_modifiers_and_key() {
        let (mods, key) = parse("ctrl+shift+t").unwrap();
        assert_eq!(mods, vec![29, 42]);
        assert!(matches!(key, Target::Char('t')));
    }

    #[test]
    fn named_keys_use_evdev_codes() {
        assert!(matches!(parse("Enter").unwrap().1, Target::Code(28)));
        assert!(matches!(parse("alt+F4").unwrap().1, Target::Code(62)));
        assert!(matches!(parse("ctrl+Left").unwrap().1, Target::Code(105)));
    }

    #[test]
    fn unknown_names_are_rejected() {
        assert!(parse("hyper+x").is_err());
        assert!(parse("ctrl+banane").is_err());
        assert!(parse("").is_err());
    }
}
