use super::{Backend, KeyEvent};
use crate::{
    config::Config,
    engine::{MouseButton, Scene},
    geometry::Rect,
};
use anyhow::{Context, Result};
use core_foundation::{
    base::{CFType, CFTypeRef, TCFType},
    string::CFString,
};
use core_graphics::{
    display::CGDisplay,
    event::{CGEvent, CGEventTapLocation, CGEventType, CGMouseButton, EventField, ScrollEventUnit},
    event_source::{CGEventSource, CGEventSourceStateID},
    geometry::{CGPoint, CGSize},
};
use objc2::rc::autoreleasepool;
use objc2_app_kit::NSWorkspace;
use std::{ffi::c_void, time::Duration};

mod ffi;
mod input;
mod overlay;
pub use overlay::initialize;

pub struct NativeBackend {
    capture: input::Capture,
    config: Config,
    buttons: [bool; 3],
}
impl NativeBackend {
    pub fn new(config: &Config) -> Result<Self> {
        Ok(Self {
            capture: input::Capture::new(leader_code(&config.leader)?)?,
            config: config.clone(),
            buttons: [false; 3],
        })
    }
    fn move_pointer(&self, point: CGPoint) -> Result<()> {
        let (kind, button) = if self.buttons[0] {
            (CGEventType::LeftMouseDragged, CGMouseButton::Left)
        } else if self.buttons[1] {
            (CGEventType::RightMouseDragged, CGMouseButton::Right)
        } else if self.buttons[2] {
            (CGEventType::OtherMouseDragged, CGMouseButton::Center)
        } else {
            (CGEventType::MouseMoved, CGMouseButton::Left)
        };
        let event = CGEvent::new_mouse_event(source()?, kind, point, button)
            .map_err(|()| anyhow::anyhow!("could not create macOS pointer event"))?;
        event.post(CGEventTapLocation::HID);
        Ok(())
    }
}
impl Backend for NativeBackend {
    fn screen_bounds(&self) -> Rect {
        autoreleasepool(|_| {
            let focus = focused_window_center()
                .or_else(|| pointer().ok())
                .unwrap_or(CGPoint::new(0.0, 0.0));
            let mut bounds = CGDisplay::main().bounds();
            for display in CGDisplay::active_displays().unwrap_or_default() {
                let r = CGDisplay::new(display).bounds();
                if self.config.span_all_monitors {
                    let left = bounds.origin.x.min(r.origin.x);
                    let top = bounds.origin.y.min(r.origin.y);
                    let right =
                        (bounds.origin.x + bounds.size.width).max(r.origin.x + r.size.width);
                    let bottom =
                        (bounds.origin.y + bounds.size.height).max(r.origin.y + r.size.height);
                    bounds.origin = CGPoint::new(left, top);
                    bounds.size = CGSize::new(right - left, bottom - top);
                } else if focus.x >= r.origin.x
                    && focus.y >= r.origin.y
                    && focus.x < r.origin.x + r.size.width
                    && focus.y < r.origin.y + r.size.height
                {
                    bounds = r;
                    break;
                }
            }
            Rect {
                x: bounds.origin.x as i32,
                y: bounds.origin.y as i32,
                width: bounds.size.width as u32,
                height: bounds.size.height as u32,
            }
        })
    }
    fn next_event(&mut self, timeout: Duration) -> Result<Option<KeyEvent>> {
        Ok(self.capture.next(timeout)?.and_then(|key| {
            key_name(key.code).map(|name| KeyEvent {
                key: name.into(),
                pressed: key.down,
            })
        }))
    }
    fn apply_config(&mut self, config: &Config) -> Result<()> {
        self.capture.set_leader(leader_code(&config.leader)?)?;
        self.config = config.clone();
        Ok(())
    }
    fn set_active(&mut self, active: bool) {
        self.capture.set_active(active);
    }
    fn show(&mut self, scene: &Scene) -> Result<()> {
        overlay::show(scene.clone(), self.config.clone());
        Ok(())
    }
    fn hide(&mut self) -> Result<()> {
        overlay::hide();
        Ok(())
    }
    fn move_to(&mut self, x: i32, y: i32) -> Result<()> {
        self.move_pointer(CGPoint::new(x as f64, y as f64))
    }
    fn move_by(&mut self, dx: i32, dy: i32) -> Result<()> {
        let p = pointer()?;
        self.move_pointer(CGPoint::new(p.x + dx as f64, p.y + dy as f64))
    }
    fn snap_to_clickable(&mut self) -> Result<()> {
        Ok(())
    } // As before, magnet snapping is Windows-only.
    fn button(&mut self, button: MouseButton, down: bool) -> Result<()> {
        let (index, button, up_kind, down_kind) = match button {
            MouseButton::Left => (
                0,
                CGMouseButton::Left,
                CGEventType::LeftMouseUp,
                CGEventType::LeftMouseDown,
            ),
            MouseButton::Right => (
                1,
                CGMouseButton::Right,
                CGEventType::RightMouseUp,
                CGEventType::RightMouseDown,
            ),
            MouseButton::Middle => (
                2,
                CGMouseButton::Center,
                CGEventType::OtherMouseUp,
                CGEventType::OtherMouseDown,
            ),
        };
        let event = CGEvent::new_mouse_event(
            source()?,
            if down { down_kind } else { up_kind },
            pointer()?,
            button,
        )
        .map_err(|()| anyhow::anyhow!("could not create macOS button event"))?;
        event.set_integer_value_field(EventField::MOUSE_EVENT_CLICK_STATE, 1);
        event.post(CGEventTapLocation::HID);
        self.buttons[index] = down;
        Ok(())
    }
    fn scroll(&mut self, amount: i32) -> Result<()> {
        let event = CGEvent::new_scroll_event(source()?, ScrollEventUnit::PIXEL, 1, amount, 0, 0)
            .map_err(|()| anyhow::anyhow!("could not create macOS scroll event"))?;
        event.post(CGEventTapLocation::HID);
        Ok(())
    }
}
impl Drop for NativeBackend {
    fn drop(&mut self) {
        for (index, button) in [MouseButton::Left, MouseButton::Right, MouseButton::Middle]
            .into_iter()
            .enumerate()
        {
            if self.buttons[index] {
                let _ = self.button(button, false);
            }
        }
        overlay::hide();
    }
}
fn source() -> Result<CGEventSource> {
    CGEventSource::new(CGEventSourceStateID::CombinedSessionState)
        .map_err(|()| anyhow::anyhow!("could not create macOS event source"))
}
fn pointer() -> Result<CGPoint> {
    CGEvent::new(source()?)
        .map(|event| event.location())
        .map_err(|()| anyhow::anyhow!("could not read macOS pointer position"))
}
fn attribute(element: &CFType, name: &str) -> Option<CFType> {
    let name = CFString::new(name);
    let mut value: CFTypeRef = std::ptr::null();
    let result = unsafe {
        ffi::AXUIElementCopyAttributeValue(
            element.as_CFTypeRef(),
            name.as_concrete_TypeRef(),
            &mut value,
        )
    };
    if result != 0 || value.is_null() {
        None
    } else {
        Some(unsafe { CFType::wrap_under_create_rule(value) })
    }
}
fn focused_window_center() -> Option<CGPoint> {
    let app = NSWorkspace::sharedWorkspace().frontmostApplication()?;
    let raw = unsafe { ffi::AXUIElementCreateApplication(app.processIdentifier()) };
    if raw.is_null() {
        return None;
    }
    let application = unsafe { CFType::wrap_under_create_rule(raw) };
    unsafe {
        ffi::AXUIElementSetMessagingTimeout(application.as_CFTypeRef(), 0.05);
    }
    let window = attribute(&application, "AXFocusedWindow")?;
    let position = attribute(&window, "AXPosition")?;
    let size = attribute(&window, "AXSize")?;
    unsafe {
        if position.type_of() != ffi::AXValueGetTypeID()
            || size.type_of() != ffi::AXValueGetTypeID()
        {
            return None;
        }
        let mut point = CGPoint::new(0.0, 0.0);
        let mut dimensions = CGSize::new(0.0, 0.0);
        if ffi::AXValueGetValue(
            position.as_CFTypeRef(),
            1,
            (&mut point as *mut CGPoint).cast::<c_void>(),
        ) == 0
            || ffi::AXValueGetValue(
                size.as_CFTypeRef(),
                2,
                (&mut dimensions as *mut CGSize).cast::<c_void>(),
            ) == 0
        {
            return None;
        }
        Some(CGPoint::new(
            point.x + dimensions.width / 2.0,
            point.y + dimensions.height / 2.0,
        ))
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
