//! `Sdl3Platform`: the `PlatformShell` implementation on top of SDL3.
//!
//! SDL3 owns the window and the events. `poll_events()` drains `event_pump.poll_iter()` and
//! translates each `SDL_Event` into an `InputEvent`, in one ordered stream.
//!
//! Keyboard mapping goes through the scancode (the physical key), not the virtual keycode.
//! Key bindings saved in `state.json` therefore do not depend on the keyboard layout.
//! Typed text comes from `TextInput` events (IME included).
//!
//! Constraints to keep in mind:
//!  1. Ownership and drop: `create_surface_unsafe` builds a `Surface<'static>` from the raw
//!     window handles. The window (`!Send`) must outlive the surface.
//!  2. Main thread on macOS: the window and the `event_pump` must live on the main thread,
//!     where SDL owns the run loop. There is one run-loop owner.
//!  3. One SDL init: the gamepad is polled from the main loop (`PadPoller`, no dedicated
//!     thread), so there is a single SDL context.
//!
//! This is the only backend of the binary. `main::run_sdl_backend` creates it with
//! `Sdl3Platform::new`.

use sdl3::event::{Event as SdlEvent, WindowEvent};
use sdl3::keyboard::{Mod, Scancode};
use sdl3::mouse::MouseButton;
use sdl3::{EventPump, Sdl, VideoSubsystem};
use sdl3::video::Window;

use crate::keycodes::Key;
use crate::platform_shell::{InputEvent, PlatformShell};
use egui::{Modifiers, PointerButton};

pub struct Sdl3Platform {
    /// SDL context, kept alive (dropping it calls `SDL_Quit`). `!Send`, so it stays on the main thread.
    _sdl: Sdl,
    /// Video subsystem. Also used for the clipboard and `text_input()`.
    video: VideoSubsystem,
    window: Window,
    event_pump: EventPump,
    /// Current modifiers, tracked from the `keymod` of keyboard events and attached to each event.
    modifiers: Modifiers,
    /// Last cursor applied (no-op when the next frame asks for the same one).
    last_cursor: Option<egui::CursorIcon>,
    /// Active SDL cursor. It must stay alive while shown (dropping it restores the default).
    cursor_keepalive: Option<sdl3::mouse::Cursor>,
    /// "Redraw requested" flag, consumed by the driver's pull loop through `wait_for_events`.
    /// It cuts the wait short on static screens.
    redraw_requested: std::cell::Cell<bool>,
    /// Events collected by `wait_for_events` (early wake-up). The next `poll_events`
    /// consumes them first to keep the stream in order.
    pending: Vec<SdlEvent>,
    /// Gamepad subsystem. The main pump updates the joystick state and `PadPoller`
    /// turns it into edge events.
    gamepad_sub: Option<sdl3::GamepadSubsystem>,
    pad: crate::gamepad::PadPoller,
    pad_out: Vec<crate::gamepad::GamepadEvent>,
}

/// Sets the program icon: the title-screen logo (`logo.png`) centered in a transparent
/// square and scaled to 512x512. It is embedded in the binary, so it does not depend on the
/// assets folder. On macOS, SDL also uses it as the Dock icon.
fn set_app_icon(window: &mut Window) {
    const PNG: &[u8] = include_bytes!("../assets/textures/ui/app_icon.png");
    let mut rgba = match image::load_from_memory(PNG) {
        Ok(img) => img.to_rgba8(),
        Err(e) => { log::warn!("Unreadable icon: {e}"); return; }
    };
    let (w, h) = rgba.dimensions();
    let surface = sdl3::surface::Surface::from_data(&mut rgba, w, h, w * 4, sdl3::pixels::PixelFormat::RGBA32);
    match surface {
        Ok(surface) => { if !window.set_icon(&surface) { log::warn!("Icon rejected by SDL"); } }
        Err(e) => log::warn!("Icon: {e}"),
    }
}

impl Sdl3Platform {
    /// Creates the SDL context and the window (size in logical points). Call it on the
    /// main thread.
    pub fn new(title: &str, width: u32, height: u32, position: Option<(i32, i32)>) -> Result<Self, Box<dyn std::error::Error>> {
        let sdl = sdl3::init()?;
        let video = sdl.video()?;

        let mut builder = video.window(title, width, height);
        match position {
            Some((x, y)) => { builder.position(x, y); }
            None => { builder.position_centered(); }
        }
        builder.resizable().high_pixel_density();
        // macOS/Metal: wgpu needs a `CAMetalLayer`, which `metal_view()` sets up.
        #[cfg(target_os = "macos")]
        builder.metal_view();
        let mut window = builder.build()?;
        set_app_icon(&mut window);

        let event_pump = sdl.event_pump()?;
        let gamepad_sub = match sdl.gamepad() {
            Ok(g) => Some(g),
            Err(e) => { log::warn!("Gamepad unavailable (SDL mode): {e}"); None }
        };
        Ok(Self { _sdl: sdl, video, window, event_pump, modifiers: Modifiers::default(), last_cursor: None, cursor_keepalive: None, redraw_requested: std::cell::Cell::new(false), pending: Vec::new(), gamepad_sub, pad: Default::default(), pad_out: Vec::new() })
    }

    /// Waits up to `budget` for an OS event. Waking early keeps input latency low on static
    /// screens, instead of sleeping blindly.
    /// The event is queued and comes out first from the next `poll_events`, so the stream
    /// stays in order. It also consumes the `request_redraw` flag: if the flag is set, it
    /// returns at once.
    /// This is an inherent method, not a trait method: pacing policy belongs to the driver.
    pub fn wait_for_events(&mut self, budget: std::time::Duration) {
        if self.redraw_requested.replace(false) {
            return;
        }
        if let Some(ev) = self.event_pump.wait_event_timeout(budget) {
            self.pending.push(ev);
        }
    }
}

impl PlatformShell for Sdl3Platform {
    fn poll_events(&mut self) -> Vec<InputEvent> {
        // Unit contract of the stream (see `platform_shell.rs`): pointer positions and pixel
        // scrolls are in physical pixels. SDL3 reports the mouse in native (logical) points,
        // so multiply by `display_scale`. Converting to egui points is the bridge's job
        // (effective DPI = native DPI x responsive zoom).
        let scale = self.window.display_scale().max(1.0);
        let mut out = Vec::new();
        let queued: Vec<SdlEvent> = self.pending.drain(..).collect();
        for ev in queued.into_iter().chain(&mut self.event_pump.poll_iter()) {
            match ev {
                SdlEvent::Quit { .. } => out.push(InputEvent::CloseRequested),
                SdlEvent::Window { win_event, .. } => match win_event {
                    WindowEvent::Resized(..) | WindowEvent::PixelSizeChanged(..) => {
                        let (pw, ph) = self.window.size_in_pixels();
                        out.push(InputEvent::Resized { pixels: (pw, ph) });
                    }
                    WindowEvent::FocusGained => out.push(InputEvent::FocusChanged(true)),
                    WindowEvent::FocusLost => out.push(InputEvent::FocusChanged(false)),
                    WindowEvent::Occluded | WindowEvent::Minimized => out.push(InputEvent::Occluded(true)),
                    WindowEvent::Restored | WindowEvent::Exposed | WindowEvent::Shown => {
                        out.push(InputEvent::Occluded(false))
                    }
                    WindowEvent::CloseRequested => out.push(InputEvent::CloseRequested),
                    _ => {}
                },
                SdlEvent::MouseMotion { x, y, .. } => {
                    out.push(InputEvent::PointerMoved { pos: (x * scale, y * scale) })
                }
                SdlEvent::MouseButtonDown { mouse_btn, x, y, .. } => {
                    if let Some(button) = map_btn(mouse_btn) {
                        out.push(InputEvent::PointerButton { pos: (x * scale, y * scale), button, pressed: true, modifiers: self.modifiers });
                    }
                }
                SdlEvent::MouseButtonUp { mouse_btn, x, y, .. } => {
                    if let Some(button) = map_btn(mouse_btn) {
                        out.push(InputEvent::PointerButton { pos: (x * scale, y * scale), button, pressed: false, modifiers: self.modifiers });
                    }
                }
                SdlEvent::MouseWheel { x, y, .. } => {
                    // The SDL wheel reports notches, which count as lines.
                    out.push(InputEvent::Scroll {
                        delta: (x, y),
                        unit: crate::platform_shell::ScrollUnit::Line,
                        modifiers: self.modifiers,
                    });
                }
                SdlEvent::KeyDown { scancode, keymod, repeat, .. } => {
                    self.modifiers = map_mods(keymod);
                    if let Some(key) = scancode.and_then(scancode_to_key) {
                        out.push(InputEvent::Key { key, pressed: true, repeat, modifiers: self.modifiers });
                    }
                }
                SdlEvent::KeyUp { scancode, keymod, .. } => {
                    self.modifiers = map_mods(keymod);
                    if let Some(key) = scancode.and_then(scancode_to_key) {
                        out.push(InputEvent::Key { key, pressed: false, repeat: false, modifiers: self.modifiers });
                    }
                }
                SdlEvent::TextInput { text, .. } => out.push(InputEvent::Text(text)),
                _ => {}
            }
        }
        // Gamepad polled in the main loop: the pump above has updated the joystick state,
        // and `PadPoller` turns it into edge events in the same ordered stream.
        if let Some(gs) = self.gamepad_sub.as_ref() {
            self.pad.poll_step(gs, &mut self.pad_out);
            for g in self.pad_out.drain(..) {
                out.push(InputEvent::Gamepad(g));
            }
        }
        out
    }

    fn create_surface(&self, instance: &wgpu::Instance) -> Result<wgpu::Surface<'static>, Box<dyn std::error::Error>> {
        // The SDL3 window is `!Send`, so `create_surface(&window)` (which needs `Send + Sync`)
        // does not compile. Use the `unsafe` path (raw handles to `Surface<'static>`).
        // The window must outlive the surface (see the module doc, point 1).
        // `from_window` can fail with a `HandleError`, which is boxed.
        let surface = unsafe {
            instance.create_surface_unsafe(wgpu::SurfaceTargetUnsafe::from_window(&self.window)?)?
        };
        Ok(surface)
    }

    fn pixel_size(&self) -> (u32, u32) {
        self.window.size_in_pixels()
    }

    fn scale_factor(&self) -> f32 {
        self.window.display_scale().max(1.0)
    }

    fn set_text_input(&mut self, active: bool) {
        let ti = self.video.text_input();
        if active { ti.start(&self.window) } else { ti.stop(&self.window) }
    }

    fn clipboard_text(&self) -> Option<String> {
        self.video.clipboard().clipboard_text().ok()
    }

    fn set_clipboard_text(&mut self, text: &str) {
        // This can fail in a remote or Wayland session, so log it.
        if let Err(e) = self.video.clipboard().set_clipboard_text(text) {
            log::warn!("Clipboard write failed: {e}");
        }
    }

    fn request_redraw(&self) {
        // Pull loop: the SDL driver redraws at a fixed rate. The flag lets the loop wake up
        // early if it is waiting.
        self.redraw_requested.set(true);
    }

    fn set_cursor_icon(&mut self, icon: egui::CursorIcon) {
        if self.last_cursor == Some(icon) { return; }
        self.last_cursor = Some(icon);
        use egui::CursorIcon as E;
        use sdl3::mouse::SystemCursor as C;
        // A subset of the SDL system cursors; everything else gets the default. `None` hides the cursor.
        let sys = match icon {
            E::None => { self._sdl.mouse().show_cursor(false); return; }
            E::PointingHand => C::Hand,
            E::Text | E::VerticalText => C::IBeam,
            E::Wait => C::Wait,
            E::Progress => C::WaitArrow,
            E::Crosshair => C::Crosshair,
            E::Move | E::AllScroll => C::SizeAll,
            E::NotAllowed | E::NoDrop => C::No,
            E::ResizeHorizontal | E::ResizeEast | E::ResizeWest => C::SizeWE,
            E::ResizeVertical | E::ResizeNorth | E::ResizeSouth => C::SizeNS,
            E::ResizeNeSw | E::ResizeNorthEast | E::ResizeSouthWest => C::SizeNESW,
            E::ResizeNwSe | E::ResizeNorthWest | E::ResizeSouthEast => C::SizeNWSE,
            _ => C::Arrow,
        };
        self._sdl.mouse().show_cursor(true);
        if let Ok(c) = sdl3::mouse::Cursor::from_system(sys) {
            c.set();
            self.cursor_keepalive = Some(c);
        }
    }

    fn set_fullscreen(&mut self, on: bool) {
        // Under X11, Wayland or RDP the window manager can refuse. Always log it: never fail silently.
        if let Err(e) = self.window.set_fullscreen(on) {
            log::warn!("Fullscreen ({on}) refused by the window manager: {e}");
        }
    }
}

/// Maps `sdl3::MouseButton` to `egui::PointerButton` (`None` for `Unknown`).
fn map_btn(b: MouseButton) -> Option<PointerButton> {
    Some(match b {
        MouseButton::Left => PointerButton::Primary,
        MouseButton::Right => PointerButton::Secondary,
        MouseButton::Middle => PointerButton::Middle,
        MouseButton::X1 => PointerButton::Extra1,
        MouseButton::X2 => PointerButton::Extra2,
        MouseButton::Unknown => return None,
    })
}

/// Labels of the printable keys for the current keyboard layout.
/// For each physical scancode, SDL gives the keycode of the layout (on AZERTY, the key at
/// `Scancode::A` is labeled "Q"). Only single-character labels are kept; named keys
/// (Space, F1...) keep their curated labels.
pub(crate) fn layout_key_labels() -> std::collections::HashMap<&'static str, String> {
    use sdl3::keyboard::Keycode;
    use sdl3::sys::keycode::SDL_Keymod;
    const PRINTABLE: [Scancode; 39] = [
        Scancode::A, Scancode::B, Scancode::C, Scancode::D, Scancode::E, Scancode::F,
        Scancode::G, Scancode::H, Scancode::I, Scancode::J, Scancode::K, Scancode::L,
        Scancode::M, Scancode::N, Scancode::O, Scancode::P, Scancode::Q, Scancode::R,
        Scancode::S, Scancode::T, Scancode::U, Scancode::V, Scancode::W, Scancode::X,
        Scancode::Y, Scancode::Z,
        Scancode::_0, Scancode::_1, Scancode::_2, Scancode::_3, Scancode::_4,
        Scancode::_5, Scancode::_6, Scancode::_7, Scancode::_8,
        Scancode::Comma, Scancode::Period, Scancode::Slash, Scancode::Semicolon,
    ];
    let mut out = std::collections::HashMap::new();
    for &sc in &PRINTABLE {
        let Some(key) = scancode_to_key(sc) else { continue };
        let Some(stable) = crate::keycodes::name_from_key(key) else { continue };
        let Some(kc) = Keycode::from_scancode(sc, SDL_Keymod(Mod::NOMOD.bits()), false) else { continue };
        let label = kc.name();
        if label.chars().count() == 1 && !label.trim().is_empty() {
            out.insert(stable, label.to_uppercase());
        }
    }
    out
}

/// Maps `sdl3::Mod` to `egui::Modifiers` (`command` is Cmd on macOS, Ctrl elsewhere).
fn map_mods(m: Mod) -> Modifiers {
    Modifiers {
        alt: m.intersects(Mod::LALTMOD | Mod::RALTMOD),
        ctrl: m.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD),
        shift: m.intersects(Mod::LSHIFTMOD | Mod::RSHIFTMOD),
        mac_cmd: m.intersects(Mod::LGUIMOD | Mod::RGUIMOD),
        command: if cfg!(target_os = "macos") {
            m.intersects(Mod::LGUIMOD | Mod::RGUIMOD)
        } else {
            m.intersects(Mod::LCTRLMOD | Mod::RCTRLMOD)
        },
    }
}

/// Maps a `sdl3::Scancode` (physical key) to `keycodes::Key`. Covers the whole bindable
/// set. Typed characters come from `TextInput`, not from this mapping.
fn scancode_to_key(s: Scancode) -> Option<Key> {
    Some(match s {
        Scancode::Up => Key::ArrowUp, Scancode::Down => Key::ArrowDown,
        Scancode::Left => Key::ArrowLeft, Scancode::Right => Key::ArrowRight,
        Scancode::A => Key::KeyA, Scancode::B => Key::KeyB, Scancode::C => Key::KeyC,
        Scancode::D => Key::KeyD, Scancode::E => Key::KeyE, Scancode::F => Key::KeyF,
        Scancode::G => Key::KeyG, Scancode::H => Key::KeyH, Scancode::I => Key::KeyI,
        Scancode::J => Key::KeyJ, Scancode::K => Key::KeyK, Scancode::L => Key::KeyL,
        Scancode::M => Key::KeyM, Scancode::N => Key::KeyN, Scancode::O => Key::KeyO,
        Scancode::P => Key::KeyP, Scancode::Q => Key::KeyQ, Scancode::R => Key::KeyR,
        Scancode::S => Key::KeyS, Scancode::T => Key::KeyT, Scancode::U => Key::KeyU,
        Scancode::V => Key::KeyV, Scancode::W => Key::KeyW, Scancode::X => Key::KeyX,
        Scancode::Y => Key::KeyY, Scancode::Z => Key::KeyZ,
        Scancode::_0 => Key::Digit0, Scancode::_1 => Key::Digit1, Scancode::_2 => Key::Digit2,
        Scancode::_3 => Key::Digit3, Scancode::_4 => Key::Digit4, Scancode::_5 => Key::Digit5,
        Scancode::_6 => Key::Digit6, Scancode::_7 => Key::Digit7, Scancode::_8 => Key::Digit8,
        Scancode::_9 => Key::Digit9,
        Scancode::Space => Key::Space, Scancode::Return => Key::Enter, Scancode::Escape => Key::Escape,
        Scancode::Tab => Key::Tab, Scancode::Backspace => Key::Backspace,
        Scancode::LShift => Key::ShiftLeft, Scancode::RShift => Key::ShiftRight,
        Scancode::LCtrl => Key::ControlLeft, Scancode::RCtrl => Key::ControlRight,
        Scancode::LAlt => Key::AltLeft, Scancode::RAlt => Key::AltRight,
        Scancode::Comma => Key::Comma, Scancode::Period => Key::Period, Scancode::Slash => Key::Slash,
        Scancode::Semicolon => Key::Semicolon, Scancode::LeftBracket => Key::BracketLeft,
        Scancode::RightBracket => Key::BracketRight, Scancode::Minus => Key::Minus,
        Scancode::Equals => Key::Equal, Scancode::Grave => Key::Backquote,
        Scancode::F1 => Key::F1, Scancode::F10 => Key::F10, Scancode::F11 => Key::F11,
        Scancode::F12 => Key::F12,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scancode_covers_bindable_set() {
        // Arrows, letters, digits, punctuation and modifiers.
        assert!(matches!(scancode_to_key(Scancode::Up), Some(Key::ArrowUp)));
        assert!(matches!(scancode_to_key(Scancode::W), Some(Key::KeyW)));
        assert!(matches!(scancode_to_key(Scancode::_1), Some(Key::Digit1)));
        assert!(matches!(scancode_to_key(Scancode::Grave), Some(Key::Backquote)));
        assert!(matches!(scancode_to_key(Scancode::LShift), Some(Key::ShiftLeft)));
        assert!(matches!(scancode_to_key(Scancode::F12), Some(Key::F12)));
    }

    #[test]
    fn mouse_button_mapping() {
        assert_eq!(map_btn(MouseButton::Left), Some(PointerButton::Primary));
        assert_eq!(map_btn(MouseButton::X1), Some(PointerButton::Extra1));
        assert_eq!(map_btn(MouseButton::Unknown), None);
    }
}
