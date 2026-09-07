use super::{Backend, KeyEvent};
use crate::{
    config::Config,
    engine::{MouseButton, Scene},
    geometry::Rect,
};
use anyhow::{Context, Result, bail};
use std::{
    ffi::{CStr, CString, c_char, c_void},
    ptr::NonNull,
    time::Duration,
};

#[repr(C)]
#[derive(Default)]
struct NativeKey {
    code: u16,
    down: bool,
}

unsafe extern "C" {
    fn kb_init(one_shot: bool);
    fn kb_create(leader: u16) -> *mut c_void;
    fn kb_error() -> *const c_char;
    fn kb_destroy(state: *mut c_void);
    fn kb_poll(state: *mut c_void, timeout: f64, key: *mut NativeKey) -> i32;
    fn kb_active(state: *mut c_void, active: bool);
    fn kb_leader(state: *mut c_void, leader: u16) -> bool;
    fn kb_bounds(span: bool) -> Rect;
    fn kb_config(
        bg: *const c_char,
        grid: *const c_char,
        text: *const c_char,
        accent: *const c_char,
        font: u32,
        opacity: u8,
        contrast: bool,
        glow: bool,
        crisp: bool,
    );
    fn kb_begin(bounds: Rect);
    fn kb_cell(bounds: Rect, label: *const c_char, matched: bool, typed: bool);
    fn kb_end();
    fn kb_hide();
    fn kb_move(state: *mut c_void, x: i32, y: i32, relative: bool);
    fn kb_button(state: *mut c_void, button: u32, down: bool);
    fn kb_scroll(amount: i32);
}

/// Called on the main thread before starting either the settings UI or one-shot mode.
pub fn initialize(one_shot: bool) {
    unsafe { kb_init(one_shot) }
}

// The event tap and drawing context must stay on their creating runtime thread.
pub struct NativeBackend {
    state: NonNull<c_void>,
    span: bool,
}
impl NativeBackend {
    pub fn new(config: &Config) -> Result<Self> {
        let leader = leader_code(&config.leader)?;
        let state = NonNull::new(unsafe { kb_create(leader) }).with_context(native_error)?;
        let mut backend = Self {
            state,
            span: config.span_all_monitors,
        };
        backend.apply_config(config)?;
        Ok(backend)
    }
}
impl Backend for NativeBackend {
    fn screen_bounds(&self) -> Rect {
        unsafe { kb_bounds(self.span) }
    }
    fn next_event(&mut self, timeout: Duration) -> Result<Option<KeyEvent>> {
        let mut key = NativeKey::default();
        match unsafe { kb_poll(self.state.as_ptr(), timeout.as_secs_f64(), &mut key) } {
            -1 => bail!("macOS keyboard capture was interrupted; restart kbmouse to resume safely"),
            0 => Ok(None),
            _ => Ok(key_name(key.code).map(|name| KeyEvent {
                key: name.into(),
                pressed: key.down,
            })),
        }
    }
    fn apply_config(&mut self, config: &Config) -> Result<()> {
        let leader = leader_code(&config.leader)?;
        let colors = [
            &config.background_color,
            &config.grid_color,
            &config.text_color,
            &config.accent_color,
        ]
        .map(|s| CString::new(s.as_str()))
        .into_iter()
        .collect::<std::result::Result<Vec<_>, _>>()?;
        unsafe {
            if !kb_leader(self.state.as_ptr(), leader) {
                bail!(native_error());
            }
            kb_config(
                colors[0].as_ptr(),
                colors[1].as_ptr(),
                colors[2].as_ptr(),
                colors[3].as_ptr(),
                config.font_size,
                config.backdrop_opacity,
                config.high_contrast_labels,
                config.label_glow,
                config.crisp_labels,
            );
        }
        self.span = config.span_all_monitors;
        Ok(())
    }
    fn set_active(&mut self, active: bool) {
        unsafe { kb_active(self.state.as_ptr(), active) }
    }
    fn show(&mut self, scene: &Scene) -> Result<()> {
        // Validate strings before locking the native graphics context.
        let labels = scene
            .cells
            .iter()
            .map(|cell| CString::new(cell.label.as_str()))
            .collect::<std::result::Result<Vec<_>, _>>()?;
        unsafe {
            kb_begin(scene.bounds);
            for (cell, label) in scene.cells.iter().zip(labels.iter()) {
                kb_cell(
                    cell.bounds,
                    label.as_ptr(),
                    cell.matched,
                    !scene.typed.is_empty(),
                );
            }
            kb_end();
        }
        Ok(())
    }
    fn hide(&mut self) -> Result<()> {
        unsafe { kb_hide() };
        Ok(())
    }
    fn move_to(&mut self, x: i32, y: i32) -> Result<()> {
        unsafe { kb_move(self.state.as_ptr(), x, y, false) };
        Ok(())
    }
    fn move_by(&mut self, dx: i32, dy: i32) -> Result<()> {
        unsafe { kb_move(self.state.as_ptr(), dx, dy, true) };
        Ok(())
    }
    fn snap_to_clickable(&mut self) -> Result<()> {
        // Like X11, macOS does not yet implement accessibility magnet snapping.
        Ok(())
    }
    fn button(&mut self, button: MouseButton, down: bool) -> Result<()> {
        let button = match button {
            MouseButton::Left => 0,
            MouseButton::Right => 1,
            MouseButton::Middle => 2,
        };
        unsafe { kb_button(self.state.as_ptr(), button, down) };
        Ok(())
    }
    fn scroll(&mut self, amount: i32) -> Result<()> {
        unsafe { kb_scroll(amount) };
        Ok(())
    }
}
impl Drop for NativeBackend {
    fn drop(&mut self) {
        unsafe { kb_destroy(self.state.as_ptr()) }
    }
}

// Physical ANSI positions, matching the Windows backend's unshifted bindings.
// Caps Lock press/release events come from IOHIDManager rather than Quartz flagsChanged.
const KEYS: &[(u16, &str)] = &[
    (0, "a"),
    (1, "s"),
    (2, "d"),
    (3, "f"),
    (4, "h"),
    (5, "g"),
    (6, "z"),
    (7, "x"),
    (8, "c"),
    (9, "v"),
    (11, "b"),
    (12, "q"),
    (13, "w"),
    (14, "e"),
    (15, "r"),
    (16, "y"),
    (17, "t"),
    (18, "1"),
    (19, "2"),
    (20, "3"),
    (21, "4"),
    (22, "6"),
    (23, "5"),
    (24, "="),
    (25, "9"),
    (26, "7"),
    (27, "-"),
    (28, "8"),
    (29, "0"),
    (30, "]"),
    (31, "o"),
    (32, "u"),
    (33, "["),
    (34, "i"),
    (35, "p"),
    (36, "enter"),
    (37, "l"),
    (38, "j"),
    (39, "'"),
    (40, "k"),
    (41, ";"),
    (42, "\\"),
    (43, ","),
    (44, "/"),
    (45, "n"),
    (46, "m"),
    (47, "."),
    (48, "tab"),
    (49, "space"),
    (50, "`"),
    (51, "backspace"),
    (53, "escape"),
    (57, "capslock"),
    (96, "f5"),
    (97, "f6"),
    (98, "f7"),
    (99, "f3"),
    (100, "f8"),
    (101, "f9"),
    (103, "f11"),
    (109, "f10"),
    (111, "f12"),
    (118, "f4"),
    (120, "f2"),
    (122, "f1"),
    (115, "home"),
    (116, "pageup"),
    (117, "delete"),
    (119, "end"),
    (121, "pagedown"),
    (123, "left"),
    (124, "right"),
    (125, "down"),
    (126, "up"),
];
fn native_error() -> String {
    // Native diagnostics are static, NUL-terminated strings owned by the backend.
    unsafe { CStr::from_ptr(kb_error()) }
        .to_string_lossy()
        .into_owned()
}
fn key_name(code: u16) -> Option<&'static str> {
    KEYS.iter()
        .find(|(key, _)| *key == code)
        .map(|(_, name)| *name)
}
fn leader_code(name: &str) -> Result<u16> {
    KEYS.iter()
        .find(|(_, key)| key.eq_ignore_ascii_case(name))
        .map(|(code, _)| *code)
        .with_context(|| format!("unsupported macOS leader '{name}'"))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn keys_round_trip_and_are_unique() {
        for (code, name) in KEYS {
            assert_eq!(leader_code(name).unwrap(), *code);
            assert_eq!(key_name(*code), Some(*name));
            assert_eq!(KEYS.iter().filter(|(c, _)| c == code).count(), 1);
        }
        assert_eq!(leader_code("F9").unwrap(), 101);
        assert_eq!(leader_code("capslock").unwrap(), 57);
        assert!(key_name(65535).is_none());
    }
    #[test]
    fn default_leader_is_supported() {
        assert_eq!(Config::default().leader, "capslock");
        assert_eq!(leader_code(&Config::default().leader).unwrap(), 57);
    }
}
