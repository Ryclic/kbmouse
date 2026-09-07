//! Small bindings for C APIs not covered by our framework crates.
//! AppKit is accessed through objc2-app-kit, not hand-written Objective-C messages.
#![allow(non_snake_case)]
use core_foundation::{
    base::{CFTypeID, CFTypeRef},
    dictionary::CFDictionaryRef,
    mach_port::CFMachPortRef,
    runloop::CFRunLoopRef,
    string::CFStringRef,
};
use core_graphics::sys::CGEventRef;
use std::ffi::{c_char, c_void};

pub type HidRef = *mut c_void;
pub type HidValueCallback = unsafe extern "C" fn(*mut c_void, i32, *mut c_void, HidRef);
pub type TapCallback =
    unsafe extern "C" fn(*mut c_void, u32, CGEventRef, *mut c_void) -> CGEventRef;

#[link(name = "ApplicationServices", kind = "framework")]
unsafe extern "C" {
    pub fn AXIsProcessTrusted() -> u8;
    pub fn AXUIElementCreateApplication(pid: i32) -> CFTypeRef;
    pub fn AXUIElementSetMessagingTimeout(element: CFTypeRef, timeout: f32) -> i32;
    pub fn AXUIElementCopyAttributeValue(
        element: CFTypeRef,
        name: CFStringRef,
        value: *mut CFTypeRef,
    ) -> i32;
    pub fn AXValueGetTypeID() -> CFTypeID;
    pub fn AXValueGetValue(value: CFTypeRef, kind: u32, result: *mut c_void) -> u8;
    pub fn CGEventTapCreate(
        location: u32,
        placement: u32,
        options: u32,
        mask: u64,
        callback: TapCallback,
        context: *mut c_void,
    ) -> CFMachPortRef;
    pub fn CGEventTapEnable(tap: CFMachPortRef, enabled: bool);
    pub fn CGEventGetFlags(event: CGEventRef) -> u64;
    pub fn CGEventSetFlags(event: CGEventRef, flags: u64);
    pub fn CGEventGetIntegerValueField(event: CGEventRef, field: u32) -> i64;
}
#[link(name = "IOKit", kind = "framework")]
unsafe extern "C" {
    pub fn IOHIDCheckAccess(request: u32) -> u32;
    pub fn IOHIDManagerCreate(allocator: *const c_void, options: u32) -> HidRef;
    pub fn IOHIDManagerSetDeviceMatching(manager: HidRef, matching: CFDictionaryRef);
    pub fn IOHIDManagerSetInputValueMatching(manager: HidRef, matching: CFDictionaryRef);
    pub fn IOHIDManagerOpen(manager: HidRef, options: u32) -> i32;
    pub fn IOHIDManagerClose(manager: HidRef, options: u32) -> i32;
    pub fn IOHIDManagerRegisterInputValueCallback(
        manager: HidRef,
        callback: HidValueCallback,
        context: *mut c_void,
    );
    pub fn IOHIDManagerRegisterDeviceRemovalCallback(
        manager: HidRef,
        callback: HidValueCallback,
        context: *mut c_void,
    );
    pub fn IOHIDManagerScheduleWithRunLoop(
        manager: HidRef,
        run_loop: CFRunLoopRef,
        mode: CFStringRef,
    );
    pub fn IOHIDManagerUnscheduleFromRunLoop(
        manager: HidRef,
        run_loop: CFRunLoopRef,
        mode: CFStringRef,
    );
    pub fn IOHIDValueGetElement(value: HidRef) -> HidRef;
    pub fn IOHIDValueGetIntegerValue(value: HidRef) -> isize;
    pub fn IOHIDElementGetUsagePage(element: HidRef) -> u32;
    pub fn IOHIDElementGetUsage(element: HidRef) -> u32;
    pub fn IOHIDElementGetDevice(element: HidRef) -> HidRef;
    pub fn IOServiceMatching(name: *const c_char) -> CFDictionaryRef;
    pub fn IOServiceGetMatchingService(port: u32, matching: CFDictionaryRef) -> u32;
    pub fn IOServiceOpen(service: u32, task: u32, kind: u32, connection: *mut u32) -> i32;
    pub fn IOObjectRelease(object: u32) -> i32;
    pub fn IOServiceClose(connection: u32) -> i32;
    pub fn IOHIDGetModifierLockState(connection: u32, selector: i32, state: *mut bool) -> i32;
    pub fn IOHIDSetModifierLockState(connection: u32, selector: i32, state: bool) -> i32;
}
unsafe extern "C" {
    pub static mach_task_self_: u32;
}
