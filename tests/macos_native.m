// These tests create local event objects only: no permission prompts, taps, or posted input.
#import "../src/platform/macos/native.m"
#include <assert.h>

static bool sendKey(KBState *s, uint16_t code, bool down) {
    CGEventRef event = CGEventCreateKeyboardEvent(NULL, code, down);
    assert(event);
    bool passed = tapCallback(NULL, down ? kCGEventKeyDown : kCGEventKeyUp, event, s) == event;
    CFRelease(event);
    return passed;
}
int main(void) {
    KBState state = {0};
    state.leader = 101;
    assert(sendKey(&state, 0, true));  // Idle typing passes through.
    assert(sendKey(&state, 0, false));
    assert(state.read == state.write);
    assert(!sendKey(&state, 101, true));
    assert(state.active);  // Capture the chord even before Rust processes the leader.
    assert(!sendKey(&state, 4, true));
    kb_active(&state, false);
    assert(!sendKey(&state, 4, false));  // Don't leak releases after leaving the layer.
    assert(!sendKey(&state, 101, false));
    assert(sendKey(&state, 0, true));
    kb_active(&state, true);
    assert(sendKey(&state, 0, false));  // Release of a key pressed before activation.
    assert(state.write == 4);
    assert(state.queue[0].code == 101 && state.queue[0].down);
    assert(state.queue[3].code == 101 && !state.queue[3].down);
    for (unsigned i = 0; i < 256; i++) sendKey(&state, 4, true);
    assert(state.overflow);  // Lost input must stop the runtime instead of leaving a held key.
    KBKey key;
    assert(kb_poll(&state, 0, &key) == -1);
    state.overflow = false;
    tapCallback(NULL, kCGEventTapDisabledByTimeout, NULL, &state);
    assert(state.overflow);
    KBState caps = {0};
    caps.leader = capsCode;
    caps.capsDevicesDown = CFSetCreateMutable(NULL, 0, &kCFTypeSetCallBacks);
    capsTransition(&caps, CFSTR("keyboard one"), true);
    assert(caps.active);
    assert(caps.write == 1 && caps.queue[0].code == 57 && caps.queue[0].down);
    capsTransition(&caps, CFSTR("keyboard one"), true); // Ignore duplicate HID reports.
    capsTransition(&caps, CFSTR("keyboard two"), true);
    capsTransition(&caps, CFSTR("keyboard one"), false);
    assert(caps.write == 1); // Still held on the second keyboard.
    capsRemoved(&caps, kIOReturnSuccess, NULL, (IOHIDDeviceRef)CFSTR("keyboard two"));
    assert(caps.write == 2 && !caps.queue[1].down); // Unplugging releases the leader.
    capsTransition(&caps, CFSTR("keyboard one"), false);
    assert(caps.write == 2); // Ignore unmatched releases.
    for (unsigned i = 0; i < 3; i++) {
        capsTransition(&caps, CFSTR("keyboard one"), true);
        capsTransition(&caps, CFSTR("keyboard one"), false);
    }
    assert(caps.write == 8); // Every tap has a press/release, regardless of lock state.
    CGEventRef flags = CGEventCreateKeyboardEvent(NULL, capsCode, true);
    CGEventSetFlags(flags, 0);
    assert(tapCallback(NULL, kCGEventFlagsChanged, flags, &caps) == NULL);
    assert(caps.write == 8); // Logical Caps Lock notifications must not double-activate.
    CGEventSetIntegerValueField(flags, kCGKeyboardEventKeycode, 56); // Shift.
    assert(tapCallback(NULL, kCGEventFlagsChanged, flags, &caps) == flags);
    assert(caps.write == 8); // Other modifiers aren't leader releases.
    assert(capsFlags(&caps, kCGEventFlagMaskAlphaShift | kCGEventFlagMaskShift) == kCGEventFlagMaskShift);
    caps.originalCapsLock = true;
    assert(capsFlags(&caps, kCGEventFlagMaskCommand) == (kCGEventFlagMaskAlphaShift | kCGEventFlagMaskCommand));
    CFRelease(flags);
    CFRelease(caps.capsDevicesDown);
    puts("macOS native input tests passed (including physical Caps Lock transitions)");
    return 0;
}
