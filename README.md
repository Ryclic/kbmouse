# kbmouse

`kbmouse` is a small keyboard-driven virtual mouse for Windows and X11 Linux.
Tap Caps Lock, type the label shown over a screen region, and the pointer jumps
there. It is inspired by [warpd](https://github.com/rvaiya/warpd).

This repository is a beta. Windows is the primary platform. The platform-neutral
engine and X11 backend are tested on Linux; the Win32 backend must still be
manually exercised on a Windows desktop before a production release.

## Controls

1. Tap `Caps Lock` to display the grid, or hold it to use Normal Mode as a
   momentary keyboard layer. Caps Lock is swallowed while kbmouse is running,
   so it does not toggle the normal Caps Lock state.
2. The grid opens on the monitor containing the currently focused window. Type
   a complete label to move to that cell.
3. In normal mode:
   - `Space`: subdivide the selected cell for a more precise jump
   - `h`, `j`, `k`, `l`: nudge left, down, up, right; hold one horizontal and
     one vertical key together for diagonal movement
   - `m`, `,`, `.`: left, middle, right click
   - `v`: begin/end a left-button drag
   - `e`, `d`: scroll up/down
   - `Esc`: return to idle

For quick mouse control, hold Caps Lock, use any Normal Mode keys, then release
Caps Lock. A short Caps Lock press without another key opens the grid. Holding
Caps Lock longer than `leader_tap_ms` without using it does nothing on release.

All controls are configurable.

## Build

Install a current stable Rust toolchain, then:

```sh
cargo build --release
cargo test
```

The executable is `target/release/kbmouse` (`kbmouse.exe` on Windows).

### Windows

Build on Windows with the MSVC Rust toolchain:

```powershell
cargo build --release
.\target\release\kbmouse.exe
```

The release executable has no console window. Start it from PowerShell with
`--verbose` while diagnosing startup problems, or use a debug build. An
unelevated kbmouse cannot control elevated applications; run it as administrator
if that is required.

To cross-compile a Windows executable from Ubuntu or WSL:

```sh
sudo apt install mingw-w64
rustup target add x86_64-pc-windows-gnu
cargo build --release --target x86_64-pc-windows-gnu
```

The result is `target/x86_64-pc-windows-gnu/release/kbmouse.exe`.

### Linux

The beta Linux backend requires an X11 session and the XTest and XFixes server
extensions. Wayland is not supported. On Wayland, use an X11 session for now;
a future layer-shell one-shot backend can make `kbmouse --hint` suitable for
compositor keybindings.

## Usage

```text
kbmouse [--hint] [--config PATH] [--verbose]
```

- `--hint` opens hint mode immediately and exits after that interaction.
- `--config` selects a nonstandard configuration file.
- `--verbose` enables debug logs.

On first launch, kbmouse creates `%APPDATA%\kbmouse\config.toml` on Windows or
`~/.config/kbmouse/config.toml` on Linux.

## Settings window and tray

Normal startup opens an egui settings window while keyboard control runs on a
background thread. The General, Appearance, and Controls pages cover the common
configuration options. Select **Save settings** to persist and immediately apply
the new configuration. Saving safely exits any active hint, movement, or drag
mode before switching settings.

The Controls page includes both the default Vim `HJKL` movement layout and an
arrow-style `IJKL` preset (`I` up, `J` left, `K` down, `L` right). Selecting a
preset updates the editable direction bindings.

On Windows, closing the settings window hides it instead of stopping kbmouse.
Left-click the kbmouse notification-area icon to reopen settings. Right-click it
for the menu containing the Quit command.
On Linux, closing the window exits the application.

## Example configuration

```toml
leader = "capslock"
hold_leader_for_normal = true
leader_tap_ms = 200
label_style = "sequences"
alphabet = "asdfghjkl;qwertyuiop"
target_cell_px = 100
backdrop_opacity = 90
background_color = "#111827"
grid_color = "#64748b"
text_color = "#ffffff"
accent_color = "#38bdf8"
high_contrast_labels = true
crisp_labels = false
label_glow = false
font_size = 22
post_hint = "normal"
exit_on_click = true
move_step = 8
hold_move_step = 24
smooth_movement = false
scroll_step = 120
span_all_monitors = false

[keys]
left = "h"
down = "j"
up = "k"
right = "l"
left_click = "m"
middle_click = ","
right_click = "."
drag = "v"
scroll_up = "e"
scroll_down = "d"
subdivide = "space"
```

Set `grid_rows` and `grid_cols` to explicit positive integers if you do not want
the automatic approximately-100-pixel cells. `post_hint` accepts `normal`,
`click`, or `exit`.

Set `label_style = "words"` or choose **Three-letter words** in the General page
to replace generated key sequences with recognizable labels such as `ace`,
`cat`, and `sun`. Word mode may reduce grid density on very large displays so
every label remains a unique three-letter word.

Enable **Smooth acceleration** under Settings → Controls for granular direction
key taps that accelerate into fluid movement when held. It is experimental and
disabled by default so the constant-speed mode remains available for users who
prefer predictable pixel movement.

## Manual beta checklist

### Windows

- Type in Notepad, summon and dismiss kbmouse, then continue typing. The overlay
  must not steal focus or lose ordinary keystrokes.
- Verify Caps Lock's state and LED do not toggle while kbmouse runs, and that
  Caps Lock works normally after kbmouse exits.
- Verify all click types, scrolling, drag release on `Esc`, and two-stage
  subdivision.
- Verify a secondary monitor to the left of the primary (negative coordinates).
- Verify mixed 100%/150% DPI monitors and an elevated app.
- Force-terminate kbmouse while the overlay is open. Windows must remove the hook
  and normal keyboard behavior must return.

### X11

- Confirm the XTest and XFixes extensions are present (`xdpyinfo -queryExtensions`).
- Verify the overlay does not receive pointer clicks and does not focus itself.
- Verify another application receives all keys while kbmouse is idle.

## Known beta limitations

- Editing `config.toml` manually still requires a restart; GUI saves apply live.
- The tray icon is currently Windows-only; there is no installer yet.
- No Wayland or macOS backend.
- X11 uses the server's core bitmap font and a solid backdrop.
- Multi-monitor selection on X11 currently uses the root screen as one desktop.
- Key movement uses operating-system key repeat rather than time-based animation.
