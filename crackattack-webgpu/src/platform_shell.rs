//! Platform layer (the shell boundary), split into three distinct layers:
//!
//! - [`PlatformShell`]: window, wgpu surface, scale/DPI, clipboard, text input, and above
//!   all a single ordered event stream per frame (`poll_events`);
//! - `ui_bridge`: `&[InputEvent]` to `egui::RawInput`;
//! - `gameplay_bridge`: `&[InputEvent]` to router events (`KeyEvent`) and gamepad events.
//!
//! Why one ordered stream (and not four separate pull outputs): egui and the router must see
//! the same logical sequence (for example focus lost, `Escape`, text, resize). Separate
//! channels could disagree on order or consumption. `poll_events()` returns the sequence,
//! and the bridges derive their views from it, in order.
//!
//! The bridges are backend-neutral: they work on `InputEvent`. Only `poll_events()` is
//! backend-specific: the SDL `poll_iter` loop emits `InputEvent` values directly.
//!
//! Redraw scheduling is not part of the trait. It belongs to the driver, which paces its
//! pull loop. The shell exposes `Occluded` and `FocusChanged` so the driver can throttle.
//!
//! The contract is fully wired: egui input (`poll_events`, `ui_bridge`), keyboard and
//! gamepad routing (`to_key_event`, `InputEvent::Gamepad`), and outputs (cursor, clipboard,
//! fullscreen, `request_redraw`, `set_text_input`).

use crate::gamepad::GamepadEvent;
use crate::keycodes::Key;
use egui::{Modifiers, PointerButton};

/// A normalized, backend-agnostic input event.
///
/// One ordered sequence per frame (window, pointer, keyboard, text, gamepad) feeds both the
/// egui bridge and the router bridge.
///
/// Unit contract: pointer positions and pixel scrolling are in physical pixels. Converting
/// them to egui points is the job of `ui_bridge::build_raw_input` alone. It divides by the
/// effective pixels-per-point (native scale times the responsive `zoom_factor`), like
/// egui-winit does. Backends must not divide by the native scale themselves.
#[derive(Debug, Clone)]
pub enum InputEvent {
    // Shell / lifecycle
    Resized { pixels: (u32, u32) },
    FocusChanged(bool),
    Occluded(bool),
    CloseRequested,
    // Pointer (positions in physical pixels, see the unit contract above)
    PointerMoved { pos: (f32, f32) },
    PointerButton { pos: (f32, f32), button: PointerButton, pressed: bool, modifiers: Modifiers },
    Scroll { delta: (f32, f32), unit: ScrollUnit, modifiers: Modifiers },
    // Keyboard / text
    Key { key: Key, pressed: bool, repeat: bool, modifiers: Modifiers },
    Text(String),
    // Gamepad
    Gamepad(GamepadEvent),
}

/// Unit of an `InputEvent::Scroll`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScrollUnit {
    /// Wheel lines. Values are floats, because SDL3 trackpads report fine-grained amounts.
    /// They are not divided by the pixels-per-point.
    Line,
}

/// A physical keyboard event for the router (`dispatch_input`).
#[derive(Debug, Clone, Copy)]
pub struct KeyEvent {
    pub key: Key,
    pub pressed: bool,
    pub repeat: bool,
}

/// The shell layer: window, surface, scale, clipboard, text, and the ordered event stream.
///
/// It contains neither the egui adaptation nor the router adaptation (those are the bridges).
pub trait PlatformShell {
    /// Returns the ordered event stream of the frame (window and input events). It is the
    /// single source of truth.
    fn poll_events(&mut self) -> Vec<InputEvent>;

    /// Creates the wgpu surface. The platform owns the window.
    ///
    /// Ownership is delicate here: the SDL3 implementation uses `create_surface_unsafe` to get
    /// a `Surface<'static>`. The error is boxed because raw window handles can fail with
    /// `HandleError` in addition to `CreateSurfaceError`.
    fn create_surface(&self, instance: &wgpu::Instance) -> Result<wgpu::Surface<'static>, Box<dyn std::error::Error>>;

    fn pixel_size(&self) -> (u32, u32);
    fn scale_factor(&self) -> f32;
    /// Turns text input (IME, typed characters) on or off. SDL3 delivers `Text` events only
    /// while it is on, so the driver turns it on while a text field takes the keyboard.
    fn set_text_input(&mut self, active: bool);
    fn clipboard_text(&self) -> Option<String>;
    fn set_clipboard_text(&mut self, text: &str);
    /// Asks the driver for a redraw. SDL raises a flag that the pull loop consumes.
    ///
    /// The pacing policy stays with the driver; this is only the "work is waiting" signal.
    fn request_redraw(&self);
    /// Sets the mouse cursor egui requested for the frame (`PlatformOutput.cursor_icon`).
    ///
    /// `CursorIcon::None` hides the cursor. Implementations cache the last value, so an
    /// unchanged value is a no-op.
    fn set_cursor_icon(&mut self, icon: egui::CursorIcon);
    fn set_fullscreen(&mut self, on: bool);
}

/// UI bridge: derives an `egui::RawInput` from the ordered stream. Backend-neutral.
pub mod ui_bridge {
    use super::*;
    use egui::{pos2, vec2, Event, MouseWheelUnit, Pos2, Rect, RawInput, ViewportId};

    /// Derives an `egui::RawInput` from the ordered stream.
    ///
    /// - `screen_px`: window size in physical pixels.
    /// - `native_ppp`: native DPI scale, set on the ROOT viewport for sharp rendering.
    /// - `effective_ppp`: native scale times the responsive `zoom_factor`. This value
    ///   converts pixels to egui points (`screen_rect`, pointer, pixel scrolling), as
    ///   `egui_winit::pixels_per_point` does. Using the native scale here would break the
    ///   responsive layout whenever the window leaves the reference size.
    pub fn build_raw_input(
        events: &[InputEvent],
        screen_px: (f32, f32),
        native_ppp: f32,
        effective_ppp: f32,
        focused: bool,
    ) -> RawInput {
        let eppp = effective_ppp.max(0.1);
        let mut raw = RawInput {
            screen_rect: Some(Rect::from_min_size(
                Pos2::ZERO,
                vec2(screen_px.0 / eppp, screen_px.1 / eppp),
            )),
            focused,
            ..Default::default()
        };
        raw.viewports.entry(ViewportId::ROOT).or_default().native_pixels_per_point = Some(native_ppp);
        let mut last_mods = Modifiers::default();
        for ev in events {
            match ev {
                InputEvent::PointerMoved { pos } => {
                    raw.events.push(Event::PointerMoved(pos2(pos.0 / eppp, pos.1 / eppp)));
                }
                InputEvent::PointerButton { pos, button, pressed, modifiers } => {
                    last_mods = *modifiers;
                    raw.events.push(Event::PointerButton {
                        pos: pos2(pos.0 / eppp, pos.1 / eppp), button: *button, pressed: *pressed, modifiers: *modifiers,
                    });
                }
                InputEvent::Scroll { delta, unit, modifiers } => {
                    last_mods = *modifiers;
                    let (unit, delta) = match unit {
                        ScrollUnit::Line => (MouseWheelUnit::Line, vec2(delta.0, delta.1)),
                    };
                    raw.events.push(Event::MouseWheel { unit, delta, modifiers: *modifiers });
                }
                InputEvent::Key { key, pressed, repeat, modifiers } => {
                    last_mods = *modifiers;
                    // No Copy/Cut/Paste synthesis here: the app has no egui `TextEdit`.
                    // All text input goes through the custom on-screen keyboard editor, which
                    // reads `Event::Text`. Pasting is deliberately unsupported there; the LAN
                    // screen has a dedicated paste button. Revisit if an egui `TextEdit` is
                    // ever added.
                    if let Some(k) = key_to_egui(*key) {
                        raw.events.push(Event::Key {
                            key: k, physical_key: None, pressed: *pressed, repeat: *repeat, modifiers: *modifiers,
                        });
                    }
                }
                InputEvent::Text(t) => raw.events.push(Event::Text(t.clone())),
                // Window and gamepad events are ignored by egui. The shell and the gameplay
                // bridge consume them.
                _ => {}
            }
        }
        raw.modifiers = last_mods;
        raw
    }

    /// Maps `keycodes::Key` to `egui::Key`.
    ///
    /// Only navigation and shortcut keys are mapped; characters arrive through `Event::Text`.
    /// Returns `None` for what egui does not model as a key (modifiers, etc.).
    fn key_to_egui(k: Key) -> Option<egui::Key> {
        use egui::Key as E;
        Some(match k {
            Key::ArrowUp => E::ArrowUp, Key::ArrowDown => E::ArrowDown,
            Key::ArrowLeft => E::ArrowLeft, Key::ArrowRight => E::ArrowRight,
            Key::Enter => E::Enter, Key::Escape => E::Escape, Key::Tab => E::Tab,
            Key::Backspace => E::Backspace, Key::Space => E::Space,
            Key::KeyA => E::A, Key::KeyB => E::B, Key::KeyC => E::C, Key::KeyD => E::D,
            Key::KeyE => E::E, Key::KeyF => E::F, Key::KeyG => E::G, Key::KeyH => E::H,
            Key::KeyI => E::I, Key::KeyJ => E::J, Key::KeyK => E::K, Key::KeyL => E::L,
            Key::KeyM => E::M, Key::KeyN => E::N, Key::KeyO => E::O, Key::KeyP => E::P,
            Key::KeyQ => E::Q, Key::KeyR => E::R, Key::KeyS => E::S, Key::KeyT => E::T,
            Key::KeyU => E::U, Key::KeyV => E::V, Key::KeyW => E::W, Key::KeyX => E::X,
            Key::KeyY => E::Y, Key::KeyZ => E::Z,
            Key::Digit0 => E::Num0, Key::Digit1 => E::Num1, Key::Digit2 => E::Num2,
            Key::Digit3 => E::Num3, Key::Digit4 => E::Num4, Key::Digit5 => E::Num5,
            Key::Digit6 => E::Num6, Key::Digit7 => E::Num7, Key::Digit8 => E::Num8, Key::Digit9 => E::Num9,
            _ => return None,
        })
    }
}

/// Gameplay / router bridge: extracts the events meant for `dispatch_input` from the ordered
/// stream.
pub mod gameplay_bridge {
    use super::*;

    /// Converts one `InputEvent` to a router `KeyEvent`.
    ///
    /// The conversion is per event, not per batch: the exclusive router re-evaluates
    /// `keyboard_goes_to_egui()` between keys. Opening the on-screen keyboard in the middle
    /// of a frame therefore routes the following keys to egui.
    pub fn to_key_event(e: &InputEvent) -> Option<KeyEvent> {
        match e {
            InputEvent::Key { key, pressed, repeat, .. } => {
                Some(KeyEvent { key: *key, pressed: *pressed, repeat: *repeat })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Order is preserved: egui and the router derive from the same ordered stream.
    #[test]
    fn ordered_stream_preserves_causality() {
        let m = Modifiers::default();
        let stream = vec![
            InputEvent::FocusChanged(false),
            InputEvent::Key { key: Key::Escape, pressed: true, repeat: false, modifiers: m },
            InputEvent::Text("a".into()),
            InputEvent::Resized { pixels: (1600, 1200) },
        ];
        // egui sees Escape before the text, in order.
        let raw = ui_bridge::build_raw_input(&stream, (1600.0, 1200.0), 2.0, 2.0, false);
        let kinds: Vec<&str> = raw.events.iter().map(|e| match e {
            egui::Event::Key { .. } => "key",
            egui::Event::Text(_) => "text",
            _ => "other",
        }).collect();
        assert_eq!(kinds, vec!["key", "text"]);
        // The router sees the same Escape (from the same stream) through the per-event
        // conversion.
        let keys: Vec<_> = stream.iter().filter_map(gameplay_bridge::to_key_event).collect();
        assert_eq!(keys.len(), 1);
        assert!(matches!(keys[0].key, Key::Escape));
    }

    /// Responsive regression guard. When the zoom is not 1, `screen_rect`, the pointer and
    /// pixel scrolling must be in egui points: physical pixels divided by the effective
    /// pixels-per-point (native times zoom), not by the native scale alone.
    #[test]
    fn coordinates_use_effective_ppp() {
        let m = Modifiers::default();
        let stream = vec![
            InputEvent::PointerMoved { pos: (300.0, 600.0) }, // physical pixels
            InputEvent::PointerButton { pos: (900.0, 1200.0), button: egui::PointerButton::Primary, pressed: true, modifiers: m },
            InputEvent::Scroll { delta: (0.0, 1.5), unit: ScrollUnit::Line, modifiers: m },
            InputEvent::Scroll { delta: (0.0, 2.0), unit: ScrollUnit::Line, modifiers: m },
        ];
        // Window of 2100x2100 physical px; native DPI 2; responsive zoom 1.5; effective 3.
        let raw = ui_bridge::build_raw_input(&stream, (2100.0, 2100.0), 2.0, 3.0, true);

        // `screen_rect` in egui points: 2100 / 3 = 700 (not 2100 / 2 = 1050, the buggy value).
        let rect = raw.screen_rect.unwrap();
        assert!((rect.width() - 700.0).abs() < 1e-4 && (rect.height() - 700.0).abs() < 1e-4);
        // The native scale stays native (sharp rendering). It is not used for coordinates.
        assert_eq!(raw.viewports[&egui::ViewportId::ROOT].native_pixels_per_point, Some(2.0));

        match &raw.events[0] {
            egui::Event::PointerMoved(p) => assert!((p.x - 100.0).abs() < 1e-4 && (p.y - 200.0).abs() < 1e-4),
            e => panic!("expected PointerMoved, got {e:?}"),
        }
        match &raw.events[1] {
            egui::Event::PointerButton { pos, .. } => assert!((pos.x - 300.0).abs() < 1e-4 && (pos.y - 400.0).abs() < 1e-4),
            e => panic!("expected PointerButton, got {e:?}"),
        }
        match &raw.events[2] {
            egui::Event::MouseWheel { unit, delta, .. } => {
                assert_eq!(*unit, egui::MouseWheelUnit::Line);
                // Wheel lines are not divided by the pixels-per-point.
                assert!((delta.y - 1.5).abs() < 1e-4);
            }
            e => panic!("expected MouseWheel, got {e:?}"),
        }
        match &raw.events[3] {
            egui::Event::MouseWheel { unit, delta, .. } => {
                assert_eq!(*unit, egui::MouseWheelUnit::Line);
                assert!((delta.y - 2.0).abs() < 1e-4);
            }
            e => panic!("expected MouseWheel, got {e:?}"),
        }
    }

}
