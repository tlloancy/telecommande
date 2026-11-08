//! Entrées via /dev/uinput.
//! Phase 3 : souris relative, clics, molette. Le clavier (phase 5) est ignoré ici.
//! Le démon ne tourne jamais en root : la règle udev donne /dev/uinput au groupe telecommande.

use evdev::uinput::VirtualDeviceBuilder;
use evdev::{AttributeSet, EventType, InputEvent as EvdevEvent, Key, RelativeAxisType};
use serde::Deserialize;
use std::io;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Messages reçus des DataChannels.
#[derive(Debug, Deserialize, PartialEq)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum InputEvent {
    Move { dx: i32, dy: i32 },
    Button { b: u8, down: bool },
    /// +dy : le contenu monte (scroll naturel, déjà inversé côté téléphone).
    Scroll { dx: i32, dy: i32 },
    /// Demande la géométrie des écrans. La réponse part sur le DataChannel.
    Hello,
    /// `name` vide : les deux écrans. Sinon la sortie X11 (`eDP-1`, `DP-2`).
    Focus { name: String },
    Key { code: u16, down: bool },
    Text { s: String },
}

/// Pixels de doigt pour un cran de molette.
const WHEEL_STEP: i32 = 40;

#[derive(Debug, Default, PartialEq)]
struct Wheel {
    x: i32,
    y: i32,
}

#[derive(Debug, PartialEq)]
enum Out {
    Rel(u16, i32),
    Key(u16, i32),
}

/// Traduit un message en événements noyau, sans ouvrir /dev/uinput.
fn translate(wheel: &mut Wheel, ev: &InputEvent) -> Vec<Out> {
    match ev {
        InputEvent::Move { dx, dy } => {
            let mut out = Vec::new();
            if *dx != 0 {
                out.push(Out::Rel(RelativeAxisType::REL_X.0, *dx));
            }
            if *dy != 0 {
                out.push(Out::Rel(RelativeAxisType::REL_Y.0, *dy));
            }
            out
        }
        InputEvent::Button { b, down } => {
            let code = match b {
                0 => Key::BTN_LEFT.code(),
                1 => Key::BTN_MIDDLE.code(),
                2 => Key::BTN_RIGHT.code(),
                _ => return Vec::new(),
            };
            vec![Out::Key(code, i32::from(*down))]
        }
        InputEvent::Scroll { dx, dy } => {
            let mut out = Vec::new();
            if let Some(n) = notch(&mut wheel.x, *dx) {
                out.push(Out::Rel(RelativeAxisType::REL_HWHEEL.0, n));
            }
            if let Some(n) = notch(&mut wheel.y, *dy) {
                out.push(Out::Rel(RelativeAxisType::REL_WHEEL.0, n));
            }
            out
        }
        InputEvent::Hello | InputEvent::Focus { .. } | InputEvent::Key { .. } | InputEvent::Text { .. } => {
            Vec::new()
        }
    }
}

fn notch(acc: &mut i32, delta: i32) -> Option<i32> {
    *acc += delta;
    let n = *acc / WHEEL_STEP;
    if n == 0 {
        return None;
    }
    *acc -= n * WHEEL_STEP;
    Some(n)
}

struct Pointer {
    dev: evdev::uinput::VirtualDevice,
    wheel: Wheel,
}

impl Pointer {
    fn open() -> io::Result<Self> {
        let mut keys = AttributeSet::<Key>::new();
        keys.insert(Key::BTN_LEFT);
        keys.insert(Key::BTN_RIGHT);
        keys.insert(Key::BTN_MIDDLE);
        let mut axes = AttributeSet::<RelativeAxisType>::new();
        axes.insert(RelativeAxisType::REL_X);
        axes.insert(RelativeAxisType::REL_Y);
        axes.insert(RelativeAxisType::REL_WHEEL);
        axes.insert(RelativeAxisType::REL_HWHEEL);
        let dev = VirtualDeviceBuilder::new()?
            .name("telecommande")
            .with_keys(&keys)?
            .with_relative_axes(&axes)?
            .build()?;
        Ok(Self { dev, wheel: Wheel::default() })
    }

    fn apply(&mut self, ev: &InputEvent) -> io::Result<()> {
        let batch: Vec<EvdevEvent> = translate(&mut self.wheel, ev)
            .into_iter()
            .map(|item| match item {
                Out::Rel(code, value) => EvdevEvent::new(EventType::RELATIVE, code, value),
                Out::Key(code, value) => EvdevEvent::new(EventType::KEY, code, value),
            })
            .collect();
        if batch.is_empty() {
            return Ok(());
        }
        // emit ajoute SYN_REPORT.
        self.dev.emit(&batch)
    }
}

static POINTER: Mutex<Option<Pointer>> = Mutex::new(None);
static OPEN_FAILED: AtomicBool = AtomicBool::new(false);

pub fn handle(ev: InputEvent) {
    if let InputEvent::Focus { name } = &ev {
        if !name.is_empty() && name != "all" {
            if let Err(err) = crate::screens::focus(name) {
                tracing::warn!("focus {name} : {err}");
            }
        }
        return;
    }
    if matches!(ev, InputEvent::Hello) {
        return;
    }
    if let InputEvent::Text { s } = &ev {
        if let Err(err) = crate::keyboard::type_text(s) {
            tracing::warn!("texte non saisi : {err}");
        }
        return;
    }
    if let InputEvent::Key { code, down } = &ev {
        if let Err(err) = crate::keyboard::stroke(*code, *down) {
            tracing::warn!("touche non saisie : {err}");
        }
        return;
    }
    let mut slot = POINTER.lock().unwrap_or_else(|e| e.into_inner());
    if slot.is_none() {
        if OPEN_FAILED.load(Ordering::Relaxed) {
            return;
        }
        match Pointer::open() {
            Ok(pointer) => {
                tracing::info!("souris virtuelle ouverte");
                *slot = Some(pointer);
            }
            Err(err) => {
                OPEN_FAILED.store(true, Ordering::Relaxed);
                tracing::error!(
                    "impossible d'ouvrir /dev/uinput ({err}). \
                     Le groupe telecommande doit pouvoir l'écrire (règle udev)."
                );
                return;
            }
        }
    }
    if let Some(pointer) = slot.as_mut() {
        if let Err(err) = pointer.apply(&ev) {
            tracing::warn!("entrée non injectée : {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn move_and_buttons_map_to_linux_codes() {
        let mut wheel = Wheel::default();
        assert_eq!(
            translate(&mut wheel, &InputEvent::Move { dx: 3, dy: -2 }),
            vec![
                Out::Rel(RelativeAxisType::REL_X.0, 3),
                Out::Rel(RelativeAxisType::REL_Y.0, -2)
            ]
        );
        assert_eq!(
            translate(&mut wheel, &InputEvent::Button { b: 0, down: true }),
            vec![Out::Key(Key::BTN_LEFT.code(), 1)]
        );
        assert_eq!(
            translate(&mut wheel, &InputEvent::Button { b: 2, down: false }),
            vec![Out::Key(Key::BTN_RIGHT.code(), 0)]
        );
        assert!(translate(&mut wheel, &InputEvent::Button { b: 9, down: true }).is_empty());
    }

    #[test]
    fn natural_scroll_accumulates_to_wheel_notches() {
        let mut wheel = Wheel::default();
        assert!(translate(&mut wheel, &InputEvent::Scroll { dx: 0, dy: 30 }).is_empty());
        assert_eq!(
            translate(&mut wheel, &InputEvent::Scroll { dx: 0, dy: 20 }),
            vec![Out::Rel(RelativeAxisType::REL_WHEEL.0, 1)]
        );
        assert_eq!(
            translate(&mut wheel, &InputEvent::Scroll { dx: -80, dy: 0 }),
            vec![Out::Rel(RelativeAxisType::REL_HWHEEL.0, -2)]
        );
        assert!(translate(
            &mut wheel,
            &InputEvent::Key { code: 1, down: true }
        )
        .is_empty());
        assert!(translate(&mut wheel, &InputEvent::Text { s: "é".into() }).is_empty());
    }
}
