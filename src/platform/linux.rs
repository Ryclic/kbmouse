use super::{Backend, KeyEvent};
use crate::{
    config::Config,
    engine::{MouseButton, Scene},
    geometry::Rect,
};
use anyhow::{Context, Result};
use std::{
    collections::HashMap,
    thread,
    time::{Duration, Instant},
};
use x11rb::{
    COPY_DEPTH_FROM_PARENT,
    connection::Connection,
    protocol::{
        Event,
        randr::ConnectionExt as _,
        shape::SK,
        xfixes::ConnectionExt as _,
        xproto::{
            self, ChangeGCAux, ChangeWindowAttributesAux, ConfigureWindowAux, ConnectionExt as _,
            CreateGCAux, CreateWindowAux, EventMask, GrabMode, KeyButMask, ModMask, Rectangle,
            WindowClass,
        },
        xtest::ConnectionExt as _,
    },
    rust_connection::RustConnection,
};

pub struct NativeBackend {
    conn: RustConnection,
    screen_num: usize,
    root: u32,
    window: u32,
    gc: u32,
    leader_keycode: u8,
    keymap: HashMap<u8, Vec<u32>>,
    active: bool,
    bounds: Rect,
    span_all_monitors: bool,
    high_contrast_labels: bool,
    label_glow: bool,
    background: u32,
    grid: u32,
    text: u32,
    accent: u32,
}

impl NativeBackend {
    pub fn new(config: &Config) -> Result<Self> {
        let (conn, screen_num) = x11rb::connect(None)
            .context("could not connect to X11 (Wayland is not supported; use an X11 session)")?;
        let screen = &conn.setup().roots[screen_num];
        let root = screen.root;
        let bounds = Rect {
            x: 0,
            y: 0,
            width: screen.width_in_pixels.into(),
            height: screen.height_in_pixels.into(),
        };

        let min = conn.setup().min_keycode;
        let max = conn.setup().max_keycode;
        let mapping = conn.get_keyboard_mapping(min, max - min + 1)?.reply()?;
        let width = mapping.keysyms_per_keycode as usize;
        let keymap: HashMap<u8, Vec<u32>> = mapping
            .keysyms
            .chunks(width)
            .enumerate()
            .map(|(i, symbols)| (min + i as u8, symbols.to_vec()))
            .collect();
        let leader_symbol = keysym_for_name(&config.leader)
            .with_context(|| format!("unsupported leader key '{}'", config.leader))?;
        let leader_keycode = keymap
            .iter()
            .find_map(|(code, symbols)| symbols.contains(&leader_symbol).then_some(*code))
            .with_context(|| format!("leader key '{}' not found in X11 keymap", config.leader))?;

        let window = conn.generate_id()?;
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            window,
            root,
            0,
            0,
            bounds.width as u16,
            bounds.height as u16,
            0,
            WindowClass::INPUT_OUTPUT,
            0,
            &CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(0x111827)
                .event_mask(EventMask::EXPOSURE),
        )?;
        let gc = conn.generate_id()?;
        conn.create_gc(
            gc,
            window,
            &CreateGCAux::new()
                .foreground(parse_color(&config.text_color))
                .background(parse_color(&config.background_color)),
        )?;

        // Make the overlay invisible to pointer hit testing.
        let region = conn.generate_id()?;
        conn.xfixes_create_region(region, &[])?;
        conn.xfixes_set_window_shape_region(window, SK::INPUT, 0, 0, region)?;
        conn.xfixes_destroy_region(region)?;

        conn.grab_key(
            false,
            root,
            ModMask::ANY,
            leader_keycode,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )?;
        conn.flush()?;
        Ok(Self {
            conn,
            screen_num,
            root,
            window,
            gc,
            leader_keycode,
            keymap,
            active: false,
            bounds,
            span_all_monitors: config.span_all_monitors,
            high_contrast_labels: config.high_contrast_labels,
            label_glow: config.label_glow,
            background: parse_color(&config.background_color),
            grid: parse_color(&config.grid_color),
            text: parse_color(&config.text_color),
            accent: parse_color(&config.accent_color),
        })
    }

    fn key_name(&self, detail: u8, state: KeyButMask) -> Option<String> {
        let symbols = self.keymap.get(&detail)?;
        let shifted = state.contains(KeyButMask::SHIFT);
        let symbol = symbols
            .get(usize::from(shifted))
            .copied()
            .filter(|value| *value != 0)
            .or_else(|| symbols.first().copied())?;
        name_for_keysym(symbol)
    }

    fn draw_scene(&self, scene: &Scene) -> Result<()> {
        self.conn
            .change_gc(self.gc, &ChangeGCAux::new().foreground(self.background))?;
        self.conn.poly_fill_rectangle(
            self.window,
            self.gc,
            &[Rectangle {
                x: 0,
                y: 0,
                width: scene.bounds.width as u16,
                height: scene.bounds.height as u16,
            }],
        )?;
        for cell in &scene.cells {
            let local_x = (cell.bounds.x - scene.bounds.x) as i16;
            let local_y = (cell.bounds.y - scene.bounds.y) as i16;
            self.conn.change_gc(
                self.gc,
                &ChangeGCAux::new().foreground(if cell.matched {
                    self.grid
                } else {
                    darken_color(self.grid)
                }),
            )?;
            self.conn.poly_rectangle(
                self.window,
                self.gc,
                &[Rectangle {
                    x: local_x,
                    y: local_y,
                    width: cell.bounds.width.min(u16::MAX.into()) as u16,
                    height: cell.bounds.height.min(u16::MAX.into()) as u16,
                }],
            )?;
            self.conn.change_gc(
                self.gc,
                &ChangeGCAux::new().foreground(if cell.matched {
                    if scene.typed.is_empty() {
                        self.text
                    } else {
                        self.accent
                    }
                } else {
                    self.grid
                }),
            )?;
            let text = cell.label.as_bytes();
            let text_x = local_x + cell.bounds.width as i16 / 2 - (text.len() as i16 * 3);
            let text_y = local_y + cell.bounds.height as i16 / 2 + 4;
            if self.high_contrast_labels && cell.matched {
                self.conn.change_gc(
                    self.gc,
                    &ChangeGCAux::new().foreground(darken_color(self.background)),
                )?;
                self.conn.poly_fill_rectangle(
                    self.window,
                    self.gc,
                    &[Rectangle {
                        x: text_x - 5,
                        y: text_y - 13,
                        width: (text.len() as u16 * 7) + 10,
                        height: 18,
                    }],
                )?;
                self.conn.change_gc(
                    self.gc,
                    &ChangeGCAux::new().foreground(if scene.typed.is_empty() {
                        self.text
                    } else {
                        self.accent
                    }),
                )?;
            }
            if self.label_glow && cell.matched {
                self.conn.change_gc(
                    self.gc,
                    &ChangeGCAux::new().foreground(darken_color(self.accent)),
                )?;
                for (offset_x, offset_y) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                    self.conn.image_text8(
                        self.window,
                        self.gc,
                        text_x + offset_x,
                        text_y + offset_y,
                        text,
                    )?;
                }
                self.conn.change_gc(
                    self.gc,
                    &ChangeGCAux::new().foreground(if scene.typed.is_empty() {
                        self.text
                    } else {
                        self.accent
                    }),
                )?;
            }
            self.conn
                .image_text8(self.window, self.gc, text_x, text_y, text)?;
        }
        self.conn.flush()?;
        Ok(())
    }
}

impl Backend for NativeBackend {
    fn screen_bounds(&self) -> Rect {
        let _ = self.screen_num;
        if self.span_all_monitors {
            self.bounds
        } else {
            self.focused_monitor_bounds().unwrap_or(self.bounds)
        }
    }

    fn next_event(&mut self, timeout: Duration) -> Result<Option<KeyEvent>> {
        let deadline = Instant::now() + timeout;
        loop {
            let Some(event) = self.conn.poll_for_event()? else {
                let now = Instant::now();
                if now >= deadline {
                    return Ok(None);
                }
                thread::sleep((deadline - now).min(Duration::from_millis(1)));
                continue;
            };
            match event {
                Event::KeyPress(event) => {
                    if let Some(key) = self.key_name(event.detail, event.state) {
                        return Ok(Some(KeyEvent { key, pressed: true }));
                    }
                }
                Event::KeyRelease(event) => {
                    if let Some(key) = self.key_name(event.detail, event.state) {
                        return Ok(Some(KeyEvent {
                            key,
                            pressed: false,
                        }));
                    }
                }
                _ => {}
            }
        }
    }

    fn apply_config(&mut self, config: &Config) -> Result<()> {
        let leader_symbol = keysym_for_name(&config.leader)
            .with_context(|| format!("unsupported leader key '{}'", config.leader))?;
        let leader_keycode = self
            .keymap
            .iter()
            .find_map(|(code, symbols)| symbols.contains(&leader_symbol).then_some(*code))
            .with_context(|| format!("leader key '{}' not found in X11 keymap", config.leader))?;
        self.conn
            .ungrab_key(self.leader_keycode, self.root, ModMask::ANY)?;
        self.conn.grab_key(
            false,
            self.root,
            ModMask::ANY,
            leader_keycode,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )?;
        self.leader_keycode = leader_keycode;
        self.span_all_monitors = config.span_all_monitors;
        self.high_contrast_labels = config.high_contrast_labels;
        self.label_glow = config.label_glow;
        self.background = parse_color(&config.background_color);
        self.grid = parse_color(&config.grid_color);
        self.text = parse_color(&config.text_color);
        self.accent = parse_color(&config.accent_color);
        self.conn.change_window_attributes(
            self.window,
            &ChangeWindowAttributesAux::new().background_pixel(self.background),
        )?;
        self.conn.flush()?;
        Ok(())
    }

    fn set_active(&mut self, active: bool) {
        if self.active == active {
            return;
        }
        self.active = active;
        if active {
            let _ = self.conn.grab_keyboard(
                false,
                self.root,
                x11rb::CURRENT_TIME,
                GrabMode::ASYNC,
                GrabMode::ASYNC,
            );
        } else {
            let _ = self.conn.ungrab_keyboard(x11rb::CURRENT_TIME);
        }
        let _ = self.conn.flush();
    }

    fn show(&mut self, scene: &Scene) -> Result<()> {
        self.conn.configure_window(
            self.window,
            &ConfigureWindowAux::new()
                .x(scene.bounds.x)
                .y(scene.bounds.y)
                .width(scene.bounds.width)
                .height(scene.bounds.height)
                .stack_mode(xproto::StackMode::ABOVE),
        )?;
        self.conn.map_window(self.window)?;
        self.draw_scene(scene)
    }

    fn hide(&mut self) -> Result<()> {
        self.conn.unmap_window(self.window)?;
        self.conn.flush()?;
        Ok(())
    }

    fn move_to(&mut self, x: i32, y: i32) -> Result<()> {
        self.conn.xtest_fake_input(
            xproto::MOTION_NOTIFY_EVENT,
            0,
            x11rb::CURRENT_TIME,
            self.root,
            x as i16,
            y as i16,
            0,
        )?;
        self.conn.flush()?;
        Ok(())
    }

    fn move_by(&mut self, dx: i32, dy: i32) -> Result<()> {
        let pointer = self.conn.query_pointer(self.root)?.reply()?;
        self.move_to(pointer.root_x as i32 + dx, pointer.root_y as i32 + dy)
    }

    fn snap_to_clickable(&mut self) -> Result<()> {
        // Linux accessibility snapping requires a future AT-SPI2 backend.
        Ok(())
    }

    fn button(&mut self, button: MouseButton, down: bool) -> Result<()> {
        let detail = match button {
            MouseButton::Left => 1,
            MouseButton::Middle => 2,
            MouseButton::Right => 3,
        };
        self.conn.xtest_fake_input(
            if down {
                xproto::BUTTON_PRESS_EVENT
            } else {
                xproto::BUTTON_RELEASE_EVENT
            },
            detail,
            x11rb::CURRENT_TIME,
            self.root,
            0,
            0,
            0,
        )?;
        self.conn.flush()?;
        Ok(())
    }

    fn scroll(&mut self, amount: i32) -> Result<()> {
        let detail = if amount > 0 { 4 } else { 5 };
        for pressed in [true, false] {
            self.conn.xtest_fake_input(
                if pressed {
                    xproto::BUTTON_PRESS_EVENT
                } else {
                    xproto::BUTTON_RELEASE_EVENT
                },
                detail,
                x11rb::CURRENT_TIME,
                self.root,
                0,
                0,
                0,
            )?;
        }
        self.conn.flush()?;
        Ok(())
    }
}

impl NativeBackend {
    fn focused_monitor_bounds(&self) -> Option<Rect> {
        let focused = self.conn.get_input_focus().ok()?.reply().ok()?.focus;
        // X11 reserves 0 for None and 1 for PointerRoot in GetInputFocus.
        if focused <= 1 {
            return None;
        }
        let geometry = self.conn.get_geometry(focused).ok()?.reply().ok()?;
        let translated = self
            .conn
            .translate_coordinates(focused, self.root, 0, 0)
            .ok()?
            .reply()
            .ok()?;
        let center_x = translated.dst_x as i32 + geometry.width as i32 / 2;
        let center_y = translated.dst_y as i32 + geometry.height as i32 / 2;
        self.conn
            .randr_get_monitors(self.root, true)
            .ok()?
            .reply()
            .ok()?
            .monitors
            .into_iter()
            .find(|monitor| {
                center_x >= monitor.x as i32
                    && center_x < monitor.x as i32 + monitor.width as i32
                    && center_y >= monitor.y as i32
                    && center_y < monitor.y as i32 + monitor.height as i32
            })
            .map(|monitor| Rect {
                x: monitor.x as i32,
                y: monitor.y as i32,
                width: monitor.width.into(),
                height: monitor.height.into(),
            })
    }
}

impl Drop for NativeBackend {
    fn drop(&mut self) {
        let _ = self.conn.ungrab_keyboard(x11rb::CURRENT_TIME);
        let _ = self
            .conn
            .ungrab_key(self.leader_keycode, self.root, ModMask::ANY);
        let _ = self.conn.destroy_window(self.window);
        let _ = self.conn.flush();
    }
}

fn parse_color(value: &str) -> u32 {
    u32::from_str_radix(value.trim_start_matches('#'), 16).unwrap_or(0xffffff)
}

fn darken_color(color: u32) -> u32 {
    (((color >> 16) & 0xff) / 2) << 16 | (((color >> 8) & 0xff) / 2) << 8 | ((color & 0xff) / 2)
}

fn keysym_for_name(name: &str) -> Option<u32> {
    match name.to_ascii_lowercase().as_str() {
        "capslock" => Some(0xffe5),
        "escape" => Some(0xff1b),
        "backspace" => Some(0xff08),
        "space" => Some(0x20),
        "f9" => Some(0xffc6),
        value if value.chars().count() == 1 => value.chars().next().map(|c| c as u32),
        _ => None,
    }
}

fn name_for_keysym(symbol: u32) -> Option<String> {
    match symbol {
        0xffe5 => Some("capslock".into()),
        0xff1b => Some("escape".into()),
        0xff08 => Some("backspace".into()),
        0x20 => Some("space".into()),
        0xffc6 => Some("f9".into()),
        0x21..=0x7e => char::from_u32(symbol).map(|c| c.to_ascii_lowercase().to_string()),
        _ => None,
    }
}
