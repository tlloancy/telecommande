//! Entrées via /dev/uinput (phase 3 : souris relative, clics, scroll ; phase 5 : clavier xkbcommon).
use serde::Deserialize;

/// Messages reçus des DataChannels. Les champs servent en phase 3, à l'injection uinput.
#[derive(Debug, Deserialize)]
#[allow(dead_code)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum InputEvent {
    Move { dx: i32, dy: i32 },
    Button { b: u8, down: bool },
    Scroll { dx: i32, dy: i32 },
    Key { code: u16, down: bool },
    Text { s: String },
}

// TODO phase 3 : VirtualDeviceBuilder (evdev), REL_X/REL_Y/REL_WHEEL, BTN_LEFT/RIGHT/MIDDLE.
// Le démon ne tourne jamais en root : règle udev + groupe fournis par l'installeur.
pub fn handle(_ev: InputEvent) {
    tracing::debug!("entrée reçue (non implémentée : phase 3)");
}
