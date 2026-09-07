use super::{ffi, overlay};
use anyhow::{Context, Result, bail, ensure};
use core_foundation::{
    base::{CFRelease, TCFType},
    dictionary::CFDictionary,
    mach_port::{CFMachPort, CFMachPortInvalidate},
    number::CFNumber,
    runloop::{CFRunLoop, CFRunLoopSource, kCFRunLoopCommonModes},
    string::CFString,
};
use core_graphics::{event::EventField, sys::CGEventRef};
use std::{
    cell::RefCell,
    collections::{HashSet, VecDeque},
    ffi::c_void,
    ptr::NonNull,
    time::Duration,
};

pub const CAPS: u16 = 57;
const ALPHA_SHIFT: u64 = 0x10000;
const KEY_DOWN: u32 = 10;
const KEY_UP: u32 = 11;
const FLAGS_CHANGED: u32 = 12;
const MAX_PENDING_KEYS: usize = 255;

#[derive(Debug, PartialEq, Eq)]
pub struct Key {
    pub code: u16,
    pub down: bool,
}

struct State {
    leader: u16,
    active: bool,
    swallowed: [bool; 128],
    caps_devices: HashSet<usize>,
    caps_lock: Option<(u32, bool)>,
    queue: VecDeque<Key>,
    failed: bool,
}
impl State {
    fn new(leader: u16) -> Self {
        Self {
            leader,
            active: false,
            swallowed: [false; 128],
            caps_devices: HashSet::new(),
            caps_lock: None,
            queue: VecDeque::new(),
            failed: false,
        }
    }
    fn enqueue(&mut self, code: u16, down: bool) {
        if self.queue.len() == MAX_PENDING_KEYS {
            self.failed = true;
        } else {
            self.queue.push_back(Key { code, down });
        }
    }
    fn caps_transition(&mut self, device: usize, down: bool) {
        let was_down = !self.caps_devices.is_empty();
        if down {
            self.caps_devices.insert(device);
        } else {
            self.caps_devices.remove(&device);
        }
        let is_down = !self.caps_devices.is_empty();
        if was_down != is_down {
            if is_down {
                self.active = true;
            }
            self.enqueue(CAPS, is_down);
        }
    }
    /// True means pass the event to the foreground application.
    fn route(&mut self, event_type: u32, code: u16) -> bool {
        if self.leader == CAPS && code == CAPS {
            return false;
        }
        if event_type == FLAGS_CHANGED {
            return true;
        }
        let down = event_type == KEY_DOWN;
        let swallowed = self
            .swallowed
            .get(usize::from(code))
            .copied()
            .unwrap_or(false);
        let consume = code == self.leader || self.active || swallowed;
        if !consume || (!down && code != self.leader && !swallowed) {
            return true;
        }
        if let Some(held) = self.swallowed.get_mut(usize::from(code)) {
            *held = down;
        }
        if code == self.leader && down {
            self.active = true;
        }
        self.enqueue(code, down);
        false
    }
}
fn caps_flags(flags: u64, locked: bool) -> u64 {
    if locked {
        flags | ALPHA_SHIFT
    } else {
        flags & !ALPHA_SHIFT
    }
}

// Callback contexts point to the boxed RefCell owned by Capture. All callbacks run on
// its creating run loop, and no RefCell borrow is held while that run loop is pumped.
unsafe extern "C" fn tap_callback(
    _: *mut c_void,
    event_type: u32,
    event: CGEventRef,
    context: *mut c_void,
) -> CGEventRef {
    let state = unsafe { &*context.cast::<RefCell<State>>() };
    let Ok(mut state) = state.try_borrow_mut() else {
        return event;
    };
    if event_type >= 0xffff_fffe || event.is_null() {
        state.failed = true;
        return event;
    }
    // Quartz owns the event. Use raw flags to preserve undocumented device bits,
    // and return NULL to suppress input (the crate's tap wrapper cannot do that).
    if let Some((connection, locked)) = state.caps_lock {
        let flags = unsafe { ffi::CGEventGetFlags(event) };
        let normalized = caps_flags(flags, locked);
        if normalized != flags {
            if unsafe { ffi::IOHIDSetModifierLockState(connection, 1, locked) } != 0 {
                state.failed = true;
            }
            unsafe {
                ffi::CGEventSetFlags(event, normalized);
            }
        }
    }
    let code =
        unsafe { ffi::CGEventGetIntegerValueField(event, EventField::KEYBOARD_EVENT_KEYCODE) }
            as u16;
    if state.route(event_type, code) {
        event
    } else {
        std::ptr::null_mut()
    }
}
unsafe extern "C" fn caps_input(
    context: *mut c_void,
    result: i32,
    _: *mut c_void,
    value: ffi::HidRef,
) {
    let state = unsafe { &*context.cast::<RefCell<State>>() };
    let Ok(mut state) = state.try_borrow_mut() else {
        return;
    };
    if result != 0 || value.is_null() {
        state.failed = true;
        return;
    }
    unsafe {
        let element = ffi::IOHIDValueGetElement(value);
        if ffi::IOHIDElementGetUsagePage(element) == 7 && ffi::IOHIDElementGetUsage(element) == 0x39
        {
            state.caps_transition(
                ffi::IOHIDElementGetDevice(element) as usize,
                ffi::IOHIDValueGetIntegerValue(value) != 0,
            );
        }
    }
}
unsafe extern "C" fn caps_removed(
    context: *mut c_void,
    _: i32,
    _: *mut c_void,
    device: ffi::HidRef,
) {
    if let Ok(mut state) = unsafe { &*context.cast::<RefCell<State>>() }.try_borrow_mut() {
        state.caps_transition(device as usize, false);
    }
}

struct CapsCapture {
    manager: NonNull<c_void>,
    connection: u32,
    original_lock: Option<bool>,
}
impl CapsCapture {
    fn new(context: *mut c_void) -> Result<Self> {
        // IOHIDRequestTypeListenEvent = 1; IOHIDAccessTypeGranted = 0.
        ensure!(
            unsafe { ffi::IOHIDCheckAccess(1) } == 0,
            "Caps Lock requires Input Monitoring. Enable kbmouse (or its launching terminal) in System Settings > Privacy & Security > Input Monitoring, then restart kbmouse"
        );
        let manager = NonNull::new(unsafe { ffi::IOHIDManagerCreate(std::ptr::null(), 0) })
            .context("could not create macOS keyboard observer")?;
        let mut capture = Self {
            manager,
            connection: 0,
            original_lock: None,
        };
        let keyboard = matching(&[("DeviceUsagePage", 1), ("DeviceUsage", 6)]);
        let caps = matching(&[("UsagePage", 7), ("Usage", 0x39)]);
        unsafe {
            ffi::IOHIDManagerSetDeviceMatching(manager.as_ptr(), keyboard.as_concrete_TypeRef());
            ffi::IOHIDManagerSetInputValueMatching(manager.as_ptr(), caps.as_concrete_TypeRef());
            ensure!(
                ffi::IOHIDManagerOpen(manager.as_ptr(), 0) == 0,
                "could not open physical keyboards for Caps Lock; check Input Monitoring permission and restart kbmouse"
            );
            let service = ffi::IOServiceGetMatchingService(
                0,
                ffi::IOServiceMatching(c"IOHIDSystem".as_ptr()),
            );
            ensure!(service != 0, "could not find macOS Caps Lock service");
            let result =
                ffi::IOServiceOpen(service, ffi::mach_task_self_, 1, &mut capture.connection);
            ffi::IOObjectRelease(service);
            ensure!(result == 0, "could not access macOS Caps Lock state");
            let mut locked = false;
            ensure!(
                ffi::IOHIDGetModifierLockState(capture.connection, 1, &mut locked) == 0,
                "could not read macOS Caps Lock state"
            );
            capture.original_lock = Some(locked);
            ffi::IOHIDManagerRegisterInputValueCallback(manager.as_ptr(), caps_input, context);
            ffi::IOHIDManagerRegisterDeviceRemovalCallback(manager.as_ptr(), caps_removed, context);
            ffi::IOHIDManagerScheduleWithRunLoop(
                manager.as_ptr(),
                CFRunLoop::get_current().as_concrete_TypeRef(),
                kCFRunLoopCommonModes,
            );
        }
        Ok(capture)
    }
}
impl Drop for CapsCapture {
    fn drop(&mut self) {
        unsafe {
            ffi::IOHIDManagerUnscheduleFromRunLoop(
                self.manager.as_ptr(),
                CFRunLoop::get_current().as_concrete_TypeRef(),
                kCFRunLoopCommonModes,
            );
            ffi::IOHIDManagerClose(self.manager.as_ptr(), 0);
            CFRelease(self.manager.as_ptr());
            if self.connection != 0 {
                if let Some(locked) = self.original_lock {
                    ffi::IOHIDSetModifierLockState(self.connection, 1, locked);
                }
                ffi::IOServiceClose(self.connection);
            }
        }
    }
}
fn matching(pairs: &[(&str, i32)]) -> CFDictionary<CFString, CFNumber> {
    CFDictionary::from_CFType_pairs(
        &pairs
            .iter()
            .map(|(k, v)| (CFString::new(k), CFNumber::from(*v)))
            .collect::<Vec<_>>(),
    )
}

pub struct Capture {
    state: Box<RefCell<State>>,
    tap: CFMachPort,
    source: CFRunLoopSource,
    caps: Option<CapsCapture>,
}
impl Capture {
    pub fn new(leader: u16) -> Result<Self> {
        ensure!(
            unsafe { ffi::AXIsProcessTrusted() } != 0,
            "macOS keyboard capture requires Accessibility. Enable kbmouse (or its launching terminal) in System Settings > Privacy & Security > Accessibility, then restart kbmouse"
        );
        let state = Box::new(RefCell::new(State::new(leader)));
        let context = (&*state as *const RefCell<State>).cast_mut().cast();
        let raw = unsafe {
            ffi::CGEventTapCreate(
                1,
                0,
                0,
                (1 << KEY_DOWN) | (1 << KEY_UP) | (1 << FLAGS_CHANGED),
                tap_callback,
                context,
            )
        };
        ensure!(
            !raw.is_null(),
            "could not capture keyboard; check Accessibility and Input Monitoring permissions, then restart kbmouse"
        );
        let tap = unsafe { CFMachPort::wrap_under_create_rule(raw) };
        let source = tap
            .create_runloop_source(0)
            .map_err(|()| anyhow::anyhow!("could not create keyboard run-loop source"))?;
        let mut capture = Self {
            state,
            tap,
            source,
            caps: None,
        };
        capture.set_leader(leader)?;
        unsafe {
            CFRunLoop::get_current().add_source(&capture.source, kCFRunLoopCommonModes);
            ffi::CGEventTapEnable(raw, true);
        }
        Ok(capture)
    }
    pub fn set_leader(&mut self, leader: u16) -> Result<()> {
        if leader == CAPS && self.caps.is_none() {
            let context = (&*self.state as *const RefCell<State>).cast_mut().cast();
            let caps = CapsCapture::new(context)?;
            self.state.borrow_mut().caps_lock =
                Some((caps.connection, caps.original_lock.unwrap_or(false)));
            self.caps = Some(caps);
        } else if leader != CAPS {
            self.state.borrow_mut().caps_lock = None;
            self.caps.take();
            self.state.borrow_mut().caps_devices.clear();
        }
        self.state.borrow_mut().leader = leader;
        Ok(())
    }
    pub fn set_active(&self, active: bool) {
        self.state.borrow_mut().active = active;
    }
    pub fn next(&mut self, timeout: Duration) -> Result<Option<Key>> {
        let empty = self.state.borrow().queue.is_empty();
        if empty {
            overlay::pump_events(timeout);
        }
        let mut state = self.state.borrow_mut();
        if state.failed {
            bail!("macOS keyboard capture was interrupted; restart kbmouse to resume safely");
        }
        Ok(state.queue.pop_front())
    }
}
impl Drop for Capture {
    fn drop(&mut self) {
        unsafe {
            ffi::CGEventTapEnable(self.tap.as_concrete_TypeRef(), false);
            CFRunLoop::get_current().remove_source(&self.source, kCFRunLoopCommonModes);
            CFMachPortInvalidate(self.tap.as_concrete_TypeRef());
        }
        // Unschedule every callback before its boxed context can be freed.
        self.caps.take();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn idle_typing_and_captured_key_releases() {
        let mut s = State::new(101);
        assert!(s.route(KEY_DOWN, 0));
        assert!(s.route(KEY_UP, 0));
        assert!(!s.route(KEY_DOWN, 101));
        assert!(s.active);
        assert!(!s.route(KEY_DOWN, 4));
        s.active = false;
        assert!(!s.route(KEY_UP, 4));
        assert!(!s.route(KEY_UP, 101));
        assert!(s.route(KEY_DOWN, 0));
        s.active = true;
        assert!(s.route(KEY_UP, 0)); // The application saw this press before activation.
        assert_eq!(s.queue.len(), 4);
    }
    #[test]
    fn caps_repeats_multiple_keyboards_and_unplug() {
        let mut s = State::new(CAPS);
        s.caps_transition(1, true);
        assert!(s.active);
        s.caps_transition(1, true);
        s.caps_transition(2, true);
        s.caps_transition(1, false);
        assert_eq!(s.queue.len(), 1);
        s.caps_transition(2, false); // Device removal uses the same release path.
        s.caps_transition(1, false);
        assert_eq!(
            s.queue.pop_front(),
            Some(Key {
                code: CAPS,
                down: true
            })
        );
        assert_eq!(
            s.queue.pop_front(),
            Some(Key {
                code: CAPS,
                down: false
            })
        );
        for _ in 0..3 {
            s.caps_transition(1, true);
            s.caps_transition(1, false);
        }
        assert_eq!(s.queue.len(), 6);
        assert!(!s.route(FLAGS_CHANGED, CAPS));
        assert!(s.route(FLAGS_CHANGED, 56));
        assert_eq!(s.queue.len(), 6);
    }
    #[test]
    fn caps_normalization_preserves_other_modifiers() {
        assert_eq!(caps_flags(ALPHA_SHIFT | 0x20000, false), 0x20000);
        assert_eq!(caps_flags(0x100000, true), ALPHA_SHIFT | 0x100000);
    }
    #[test]
    fn overflow_and_disabled_taps_stop_capture() {
        let mut s = State::new(101);
        for _ in 0..256 {
            s.route(KEY_DOWN, 101);
        }
        assert!(s.failed);
        assert_eq!(s.queue.len(), MAX_PENDING_KEYS);
        let state = RefCell::new(State::new(CAPS));
        unsafe {
            tap_callback(
                std::ptr::null_mut(),
                0xffff_fffe,
                std::ptr::null_mut(),
                (&state as *const RefCell<State>).cast_mut().cast(),
            );
        }
        assert!(state.borrow().failed);
    }
}
