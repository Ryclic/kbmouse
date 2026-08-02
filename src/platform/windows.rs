use super::{Backend, KeyEvent};
use crate::{
    config::Config,
    engine::{MouseButton, Scene},
    geometry::Rect,
};
use anyhow::{Context, Result, anyhow, bail};
use crossbeam_channel::{Receiver, RecvTimeoutError, Sender, unbounded};
use std::{
    mem::{size_of, zeroed},
    ptr::{null, null_mut},
    sync::{
        Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};
use windows::Win32::{
    Foundation::POINT as UiPoint,
    System::Com::{
        CLSCTX_INPROC_SERVER, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx,
        CoUninitialize,
    },
    UI::Accessibility::{
        CUIAutomation, IUIAutomation, IUIAutomationElement, UIA_ButtonControlTypeId,
        UIA_CheckBoxControlTypeId, UIA_ComboBoxControlTypeId, UIA_EditControlTypeId,
        UIA_HyperlinkControlTypeId, UIA_ListItemControlTypeId, UIA_MenuItemControlTypeId,
        UIA_RadioButtonControlTypeId, UIA_SliderControlTypeId, UIA_SpinnerControlTypeId,
        UIA_SplitButtonControlTypeId, UIA_TabItemControlTypeId, UIA_ThumbControlTypeId,
        UIA_TreeItemControlTypeId,
    },
};
use windows_sys::Win32::{
    Foundation::{COLORREF, GetLastError, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM},
    Graphics::Gdi::{
        BeginPaint, CreateFontW, CreatePen, CreateSolidBrush, DEFAULT_CHARSET, DEFAULT_PITCH,
        DT_CENTER, DT_SINGLELINE, DT_VCENTER, DeleteObject, DrawTextW, EndPaint, FF_DONTCARE,
        FW_BOLD, FillRect, GetDC, GetMonitorInfoW, LineTo, MONITOR_DEFAULTTONEAREST, MONITORINFO,
        MonitorFromPoint, MoveToEx, NONANTIALIASED_QUALITY, OUT_DEFAULT_PRECIS, PAINTSTRUCT,
        PS_SOLID, ReleaseDC, SelectObject, SetBkMode, SetTextColor, TRANSPARENT,
    },
    System::{LibraryLoader::GetModuleHandleW, Threading::GetCurrentThreadId},
    UI::{
        HiDpi::{DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, SetProcessDpiAwarenessContext},
        Input::KeyboardAndMouse::{
            INPUT, INPUT_0, INPUT_MOUSE, MOUSEEVENTF_ABSOLUTE, MOUSEEVENTF_LEFTDOWN,
            MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE,
            MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP, MOUSEEVENTF_VIRTUALDESK, MOUSEEVENTF_WHEEL,
            MOUSEINPUT, SendInput, VIRTUAL_KEY, VK_BACK, VK_CAPITAL, VK_ESCAPE, VK_F1, VK_F9,
            VK_SPACE,
        },
        WindowsAndMessaging::{
            CallNextHookEx, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
            GetCursorPos, GetForegroundWindow, GetMessageW, GetSystemMetrics, HWND_TOPMOST,
            LLKHF_INJECTED, LWA_ALPHA, LWA_COLORKEY, MSG, PM_REMOVE, PeekMessageW,
            PostThreadMessageW, RegisterClassW, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
            SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_HIDE, SW_SHOWNOACTIVATE, SWP_NOACTIVATE,
            SWP_SHOWWINDOW, SetLayeredWindowAttributes, SetWindowPos, SetWindowsHookExW,
            ShowWindow, TranslateMessage, UnhookWindowsHookEx, WH_KEYBOARD_LL, WM_KEYDOWN,
            WM_KEYUP, WM_QUIT, WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSW, WS_EX_LAYERED,
            WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW, WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
        },
    },
};

static EVENT_SENDER: OnceLock<Mutex<Option<Sender<KeyEvent>>>> = OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);
static LEADER_DOWN: AtomicBool = AtomicBool::new(false);
static LEADER_VK: AtomicU32 = AtomicU32::new(0);
static HOOK_RUNNING: AtomicBool = AtomicBool::new(false);

pub struct NativeBackend {
    receiver: Receiver<KeyEvent>,
    hook_thread: Option<JoinHandle<()>>,
    hook_thread_id: u32,
    window: HWND,
    label_window: HWND,
    span_all_monitors: bool,
    background: COLORREF,
    grid: COLORREF,
    text: COLORREF,
    accent: COLORREF,
    opacity: u8,
    font_size: u32,
    high_contrast_labels: bool,
    crisp_labels: bool,
    label_glow: bool,
    magnet_enabled: bool,
    magnet_radius: i32,
    automation: Option<IUIAutomation>,
    com_initialized: bool,
}

impl NativeBackend {
    pub fn new(config: &Config) -> Result<Self> {
        if HOOK_RUNNING
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            bail!("a Windows keyboard hook is already running");
        }

        let result = Self::create(config);
        if result.is_err() {
            HOOK_RUNNING.store(false, Ordering::Release);
            *sender_slot().lock().unwrap_or_else(|e| e.into_inner()) = None;
        }
        result
    }

    fn create(config: &Config) -> Result<Self> {
        // This must happen before creating any windows or querying monitor geometry.
        unsafe {
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        }
        let leader = vk_for_name(&config.leader)
            .with_context(|| format!("unsupported leader key '{}'", config.leader))?;
        LEADER_VK.store(leader as u32, Ordering::Release);
        LEADER_DOWN.store(false, Ordering::Release);
        ACTIVE.store(false, Ordering::Release);

        let (sender, receiver) = unbounded();
        *sender_slot().lock().unwrap_or_else(|e| e.into_inner()) = Some(sender);

        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let hook_thread = thread::Builder::new()
            .name("kbmouse-keyboard-hook".into())
            .spawn(move || hook_thread_main(ready_tx))
            .context("failed to spawn Windows keyboard hook thread")?;
        let hook_thread_id = match ready_rx
            .recv()
            .context("keyboard hook thread exited during startup")?
        {
            Ok(id) => id,
            Err(message) => {
                let _ = hook_thread.join();
                bail!("{message}");
            }
        };

        let instance = unsafe { GetModuleHandleW(null()) };
        if instance.is_null() {
            stop_hook_thread(hook_thread_id, hook_thread);
            return Err(last_error("GetModuleHandleW failed"));
        }

        let class_name = wide("kbmouse.overlay");
        let window_name = wide("kbmouse");
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: class_name.as_ptr(),
            ..unsafe { zeroed() }
        };
        if unsafe { RegisterClassW(&class) } == 0 {
            let error = unsafe { GetLastError() };
            // ERROR_CLASS_ALREADY_EXISTS is harmless for a second construction in-process.
            if error != 1410 {
                stop_hook_thread(hook_thread_id, hook_thread);
                return Err(anyhow!("RegisterClassW failed (Win32 error {error})"));
            }
        }

        let window = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                window_name.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            )
        };
        if window.is_null() {
            stop_hook_thread(hook_thread_id, hook_thread);
            return Err(last_error("CreateWindowExW failed"));
        }
        if unsafe { SetLayeredWindowAttributes(window, 0, config.backdrop_opacity, LWA_ALPHA) } == 0
        {
            unsafe { DestroyWindow(window) };
            stop_hook_thread(hook_thread_id, hook_thread);
            return Err(last_error("SetLayeredWindowAttributes failed"));
        }
        let label_window = unsafe {
            CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOOLWINDOW
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE,
                class_name.as_ptr(),
                window_name.as_ptr(),
                WS_POPUP,
                0,
                0,
                0,
                0,
                null_mut(),
                null_mut(),
                instance,
                null(),
            )
        };
        if label_window.is_null() {
            unsafe { DestroyWindow(window) };
            stop_hook_thread(hook_thread_id, hook_thread);
            return Err(last_error("failed to create label overlay"));
        }
        let label_opacity = if config.crisp_labels {
            255
        } else {
            config.backdrop_opacity
        };
        if unsafe {
            SetLayeredWindowAttributes(label_window, 0, label_opacity, LWA_COLORKEY | LWA_ALPHA)
        } == 0
        {
            unsafe {
                DestroyWindow(label_window);
                DestroyWindow(window);
            }
            stop_hook_thread(hook_thread_id, hook_thread);
            return Err(last_error("failed to configure label overlay"));
        }
        let (automation, com_initialized) = initialize_automation();

        Ok(Self {
            receiver,
            hook_thread: Some(hook_thread),
            hook_thread_id,
            window,
            label_window,
            span_all_monitors: config.span_all_monitors,
            background: parse_color(&config.background_color, 0x111827),
            grid: parse_color(&config.grid_color, 0x64748b),
            text: parse_color(&config.text_color, 0xffffff),
            accent: parse_color(&config.accent_color, 0x38bdf8),
            opacity: config.backdrop_opacity,
            font_size: config.font_size,
            high_contrast_labels: config.high_contrast_labels,
            crisp_labels: config.crisp_labels,
            label_glow: config.label_glow,
            magnet_enabled: config.magnet_enabled,
            magnet_radius: config.magnet_radius,
            automation,
            com_initialized,
        })
    }

    fn draw_scene(&self, scene: &Scene) -> Result<()> {
        let dc = unsafe { GetDC(self.window) };
        if dc.is_null() {
            return Err(last_error("GetDC failed"));
        }
        let label_dc = unsafe { GetDC(self.label_window) };
        if label_dc.is_null() {
            unsafe { ReleaseDC(self.window, dc) };
            return Err(last_error("failed to get label overlay DC"));
        }

        unsafe {
            let background = CreateSolidBrush(self.background);
            let transparent = CreateSolidBrush(0);
            let area = RECT {
                left: 0,
                top: 0,
                right: scene.bounds.width as i32,
                bottom: scene.bounds.height as i32,
            };
            FillRect(dc, &area, background);
            FillRect(label_dc, &area, transparent);
            DeleteObject(background);
            DeleteObject(transparent);
            let text_dc = if self.crisp_labels { label_dc } else { dc };

            let font = CreateFontW(
                -(self.font_size as i32),
                0,
                0,
                0,
                if self.high_contrast_labels {
                    900
                } else {
                    FW_BOLD as i32
                },
                0,
                0,
                0,
                DEFAULT_CHARSET.into(),
                OUT_DEFAULT_PRECIS.into(),
                0,
                if self.crisp_labels {
                    NONANTIALIASED_QUALITY.into()
                } else {
                    0
                },
                (DEFAULT_PITCH | FF_DONTCARE).into(),
                null(),
            );
            let old_font = if font.is_null() {
                null_mut()
            } else {
                SelectObject(text_dc, font)
            };
            SetBkMode(text_dc, TRANSPARENT as i32);

            for cell in &scene.cells {
                let left = cell.bounds.x - scene.bounds.x;
                let top = cell.bounds.y - scene.bounds.y;
                let right = left + cell.bounds.width as i32;
                let bottom = top + cell.bounds.height as i32;
                let color = if cell.matched {
                    self.grid
                } else {
                    darken(self.grid)
                };
                let pen = CreatePen(PS_SOLID, 1, color);
                let old_pen = SelectObject(dc, pen);
                MoveToEx(dc, left, top, null_mut());
                LineTo(dc, right, top);
                LineTo(dc, right, bottom);
                LineTo(dc, left, bottom);
                LineTo(dc, left, top);
                SelectObject(dc, old_pen);
                DeleteObject(pen);

                if self.high_contrast_labels && cell.matched {
                    let badge_width =
                        (self.font_size as i32 * cell.label.chars().count() as i32 * 2 / 3) + 16;
                    let badge_height = self.font_size as i32 + 10;
                    let center_x = (left + right) / 2;
                    let center_y = (top + bottom) / 2;
                    let badge = RECT {
                        left: center_x - badge_width / 2,
                        top: center_y - badge_height / 2,
                        right: center_x + badge_width / 2,
                        bottom: center_y + badge_height / 2,
                    };
                    let badge_brush = CreateSolidBrush(parse_color("#020617", 0x020617));
                    FillRect(text_dc, &badge, badge_brush);
                    DeleteObject(badge_brush);
                }

                let text_color = if cell.matched && !scene.typed.is_empty() {
                    self.accent
                } else if cell.matched {
                    self.text
                } else {
                    self.grid
                };
                let label = wide_without_nul(&cell.label);
                let mut label_rect = RECT {
                    left,
                    top,
                    right,
                    bottom,
                };
                if self.label_glow && cell.matched {
                    SetTextColor(text_dc, darken(self.accent));
                    for (offset_x, offset_y) in [(-1, 0), (1, 0), (0, -1), (0, 1)] {
                        let mut glow_rect = RECT {
                            left: left + offset_x,
                            top: top + offset_y,
                            right: right + offset_x,
                            bottom: bottom + offset_y,
                        };
                        DrawTextW(
                            text_dc,
                            label.as_ptr(),
                            label.len() as i32,
                            &mut glow_rect,
                            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                        );
                    }
                }
                SetTextColor(text_dc, text_color);
                DrawTextW(
                    text_dc,
                    label.as_ptr(),
                    label.len() as i32,
                    &mut label_rect,
                    DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                );
            }
            if !old_font.is_null() {
                SelectObject(text_dc, old_font);
            }
            if !font.is_null() {
                DeleteObject(font);
            }
            ReleaseDC(self.window, dc);
            ReleaseDC(self.label_window, label_dc);
        }
        Ok(())
    }

    fn send_mouse(&self, flags: u32, dx: i32, dy: i32, data: u32) -> Result<()> {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        if unsafe { SendInput(1, &input, size_of::<INPUT>() as i32) } != 1 {
            return Err(last_error("SendInput failed"));
        }
        Ok(())
    }
}

impl Backend for NativeBackend {
    fn screen_bounds(&self) -> Rect {
        if self.span_all_monitors {
            virtual_screen_bounds()
        } else {
            focused_monitor_bounds()
                .or_else(cursor_monitor_bounds)
                .unwrap_or_else(virtual_screen_bounds)
        }
    }

    fn next_event(&mut self, timeout: Duration) -> Result<Option<KeyEvent>> {
        // The overlay belongs to this thread, so service its queue while waiting
        // for hook events. This keeps Windows from marking it unresponsive.
        let mut message: MSG = unsafe { zeroed() };
        while unsafe { PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) } != 0 {
            unsafe {
                TranslateMessage(&message);
                DispatchMessageW(&message);
            }
        }
        match self.receiver.recv_timeout(timeout) {
            Ok(event) => Ok(Some(event)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => {
                Err(anyhow!("Windows keyboard hook channel disconnected"))
            }
        }
    }

    fn apply_config(&mut self, config: &Config) -> Result<()> {
        let leader = vk_for_name(&config.leader)
            .with_context(|| format!("unsupported leader key '{}'", config.leader))?;
        LEADER_VK.store(leader as u32, Ordering::Release);
        LEADER_DOWN.store(false, Ordering::Release);
        self.span_all_monitors = config.span_all_monitors;
        self.background = parse_color(&config.background_color, 0x111827);
        self.grid = parse_color(&config.grid_color, 0x64748b);
        self.text = parse_color(&config.text_color, 0xffffff);
        self.accent = parse_color(&config.accent_color, 0x38bdf8);
        self.opacity = config.backdrop_opacity;
        self.font_size = config.font_size;
        self.high_contrast_labels = config.high_contrast_labels;
        self.crisp_labels = config.crisp_labels;
        self.label_glow = config.label_glow;
        self.magnet_enabled = config.magnet_enabled;
        self.magnet_radius = config.magnet_radius;
        if unsafe { SetLayeredWindowAttributes(self.window, 0, self.opacity, LWA_ALPHA) } == 0 {
            return Err(last_error("failed to update overlay opacity"));
        }
        let label_opacity = if self.crisp_labels { 255 } else { self.opacity };
        if unsafe {
            SetLayeredWindowAttributes(
                self.label_window,
                0,
                label_opacity,
                LWA_COLORKEY | LWA_ALPHA,
            )
        } == 0
        {
            return Err(last_error("failed to update label opacity"));
        }
        Ok(())
    }

    fn set_active(&mut self, active: bool) {
        ACTIVE.store(active, Ordering::Release);
    }

    fn show(&mut self, scene: &Scene) -> Result<()> {
        let layered =
            unsafe { SetLayeredWindowAttributes(self.window, 0, self.opacity, LWA_ALPHA) };
        let positioned = unsafe {
            SetWindowPos(
                self.window,
                HWND_TOPMOST,
                scene.bounds.x,
                scene.bounds.y,
                scene.bounds.width as i32,
                scene.bounds.height as i32,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        };
        let labels_positioned = unsafe {
            SetWindowPos(
                self.label_window,
                HWND_TOPMOST,
                scene.bounds.x,
                scene.bounds.y,
                scene.bounds.width as i32,
                scene.bounds.height as i32,
                SWP_NOACTIVATE | SWP_SHOWWINDOW,
            )
        };
        if layered == 0 || positioned == 0 || labels_positioned == 0 {
            return Err(last_error("failed to show overlay window"));
        }
        unsafe {
            ShowWindow(self.window, SW_SHOWNOACTIVATE);
            ShowWindow(self.label_window, SW_SHOWNOACTIVATE);
        }
        self.draw_scene(scene)
    }

    fn hide(&mut self) -> Result<()> {
        unsafe {
            ShowWindow(self.label_window, SW_HIDE);
            ShowWindow(self.window, SW_HIDE);
        }
        Ok(())
    }

    fn move_to(&mut self, x: i32, y: i32) -> Result<()> {
        let bounds = virtual_screen_bounds();
        let max_x = bounds.width.saturating_sub(1).max(1) as i64;
        let max_y = bounds.height.saturating_sub(1).max(1) as i64;
        let local_x = (x - bounds.x).clamp(0, max_x as i32) as i64;
        let local_y = (y - bounds.y).clamp(0, max_y as i32) as i64;
        let normalized_x = (local_x * 65_535 / max_x) as i32;
        let normalized_y = (local_y * 65_535 / max_y) as i32;
        self.send_mouse(
            MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK,
            normalized_x,
            normalized_y,
            0,
        )
    }

    fn move_by(&mut self, dx: i32, dy: i32) -> Result<()> {
        self.send_mouse(MOUSEEVENTF_MOVE, dx, dy, 0)
    }

    fn snap_to_clickable(&mut self) -> Result<()> {
        if !self.magnet_enabled {
            return Ok(());
        }
        let Some(automation) = self.automation.as_ref() else {
            return Ok(());
        };
        let mut cursor: POINT = unsafe { zeroed() };
        if unsafe { GetCursorPos(&mut cursor) } == 0 {
            return Err(last_error("GetCursorPos failed"));
        }
        if let Some((x, y)) = find_magnet_target(automation, cursor.x, cursor.y, self.magnet_radius)
        {
            self.move_to(x, y)?;
        }
        Ok(())
    }

    fn button(&mut self, button: MouseButton, down: bool) -> Result<()> {
        let flags = match (button, down) {
            (MouseButton::Left, true) => MOUSEEVENTF_LEFTDOWN,
            (MouseButton::Left, false) => MOUSEEVENTF_LEFTUP,
            (MouseButton::Middle, true) => MOUSEEVENTF_MIDDLEDOWN,
            (MouseButton::Middle, false) => MOUSEEVENTF_MIDDLEUP,
            (MouseButton::Right, true) => MOUSEEVENTF_RIGHTDOWN,
            (MouseButton::Right, false) => MOUSEEVENTF_RIGHTUP,
        };
        self.send_mouse(flags, 0, 0, 0)
    }

    fn scroll(&mut self, amount: i32) -> Result<()> {
        self.send_mouse(MOUSEEVENTF_WHEEL, 0, 0, amount as u32)
    }
}

impl Drop for NativeBackend {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::Release);
        *sender_slot().lock().unwrap_or_else(|e| e.into_inner()) = None;
        unsafe {
            DestroyWindow(self.label_window);
            DestroyWindow(self.window);
        }
        if let Some(thread) = self.hook_thread.take() {
            stop_hook_thread(self.hook_thread_id, thread);
        }
        self.automation.take();
        if self.com_initialized {
            unsafe { CoUninitialize() };
        }
        HOOK_RUNNING.store(false, Ordering::Release);
    }
}

fn initialize_automation() -> (Option<IUIAutomation>, bool) {
    let result = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if result.is_err() {
        tracing::warn!("Windows UI Automation unavailable: COM initialization failed");
        return (None, false);
    }
    let automation =
        unsafe { CoCreateInstance::<_, IUIAutomation>(&CUIAutomation, None, CLSCTX_INPROC_SERVER) };
    match automation {
        Ok(automation) => (Some(automation), true),
        Err(error) => {
            tracing::warn!(%error, "Windows UI Automation unavailable");
            unsafe { CoUninitialize() };
            (None, false)
        }
    }
}

fn find_magnet_target(
    automation: &IUIAutomation,
    cursor_x: i32,
    cursor_y: i32,
    radius: i32,
) -> Option<(i32, i32)> {
    let walker = unsafe { automation.ControlViewWalker().ok()? };
    let step = (radius / 4).clamp(8, 20);
    let mut offsets = vec![0, -radius, radius];
    let mut offset = -radius;
    while offset <= radius {
        offsets.push(offset);
        offset += step;
    }
    offsets.sort_unstable();
    offsets.dedup();

    let radius_squared = i64::from(radius) * i64::from(radius);
    let mut best: Option<(i64, i32, i32)> = None;
    for &dy in &offsets {
        for &dx in &offsets {
            if i64::from(dx) * i64::from(dx) + i64::from(dy) * i64::from(dy) > radius_squared {
                continue;
            }
            let point = UiPoint {
                x: cursor_x + dx,
                y: cursor_y + dy,
            };
            let Ok(element) = (unsafe { automation.ElementFromPoint(point) }) else {
                continue;
            };
            let Some(rect) = clickable_ancestor(element, &walker) else {
                continue;
            };
            if rect.right <= rect.left || rect.bottom <= rect.top {
                continue;
            }

            let nearest_x = cursor_x.clamp(rect.left, rect.right - 1);
            let nearest_y = cursor_y.clamp(rect.top, rect.bottom - 1);
            let distance_x = i64::from(nearest_x - cursor_x);
            let distance_y = i64::from(nearest_y - cursor_y);
            let distance_squared = distance_x * distance_x + distance_y * distance_y;
            if distance_squared > radius_squared
                || best.is_some_and(|(score, _, _)| score <= distance_squared)
            {
                continue;
            }

            let center_x = rect.left + (rect.right - rect.left) / 2;
            let center_y = rect.top + (rect.bottom - rect.top) / 2;
            let center_dx = i64::from(center_x - cursor_x);
            let center_dy = i64::from(center_y - cursor_y);
            let (target_x, target_y) =
                if center_dx * center_dx + center_dy * center_dy <= radius_squared {
                    (center_x, center_y)
                } else {
                    (
                        if rect.right - rect.left > 4 {
                            nearest_x.clamp(rect.left + 2, rect.right - 3)
                        } else {
                            center_x
                        },
                        if rect.bottom - rect.top > 4 {
                            nearest_y.clamp(rect.top + 2, rect.bottom - 3)
                        } else {
                            center_y
                        },
                    )
                };
            best = Some((distance_squared, target_x, target_y));
        }
    }
    best.map(|(_, x, y)| (x, y))
}

fn clickable_ancestor(
    mut element: IUIAutomationElement,
    walker: &windows::Win32::UI::Accessibility::IUIAutomationTreeWalker,
) -> Option<windows::Win32::Foundation::RECT> {
    for _ in 0..5 {
        let enabled =
            unsafe { element.CurrentIsEnabled().ok() }.is_some_and(|enabled| enabled.as_bool());
        let visible = unsafe { element.CurrentIsOffscreen().ok() }
            .is_some_and(|offscreen| !offscreen.as_bool());
        let control_type = unsafe { element.CurrentControlType().ok() };
        if enabled && visible && control_type.is_some_and(is_clickable_control) {
            return unsafe { element.CurrentBoundingRectangle().ok() };
        }
        element = unsafe { walker.GetParentElement(&element).ok()? };
    }
    None
}

fn is_clickable_control(
    control_type: windows::Win32::UI::Accessibility::UIA_CONTROLTYPE_ID,
) -> bool {
    [
        UIA_ButtonControlTypeId,
        UIA_CheckBoxControlTypeId,
        UIA_ComboBoxControlTypeId,
        UIA_EditControlTypeId,
        UIA_HyperlinkControlTypeId,
        UIA_ListItemControlTypeId,
        UIA_MenuItemControlTypeId,
        UIA_RadioButtonControlTypeId,
        UIA_SliderControlTypeId,
        UIA_SpinnerControlTypeId,
        UIA_SplitButtonControlTypeId,
        UIA_TabItemControlTypeId,
        UIA_ThumbControlTypeId,
        UIA_TreeItemControlTypeId,
    ]
    .contains(&control_type)
}

fn sender_slot() -> &'static Mutex<Option<Sender<KeyEvent>>> {
    EVENT_SENDER.get_or_init(|| Mutex::new(None))
}

fn hook_thread_main(ready: mpsc::SyncSender<std::result::Result<u32, String>>) {
    let thread_id = unsafe { GetCurrentThreadId() };
    // Creating the queue before reporting readiness makes PostThreadMessageW reliable.
    let mut message: MSG = unsafe { zeroed() };
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::PeekMessageW(
            &mut message,
            null_mut(),
            0,
            0,
            0,
        );
    }
    let hook = unsafe { SetWindowsHookExW(WH_KEYBOARD_LL, Some(keyboard_hook), null_mut(), 0) };
    if hook.is_null() {
        let error = unsafe { GetLastError() };
        let _ = ready.send(Err(format!(
            "SetWindowsHookExW failed (Win32 error {error})"
        )));
        return;
    }
    if ready.send(Ok(thread_id)).is_err() {
        unsafe { UnhookWindowsHookEx(hook) };
        return;
    }

    loop {
        let status = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        if status <= 0 {
            break;
        }
        unsafe {
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    unsafe { UnhookWindowsHookEx(hook) };
}

fn stop_hook_thread(thread_id: u32, thread: JoinHandle<()>) {
    unsafe { PostThreadMessageW(thread_id, WM_QUIT, 0, 0) };
    let _ = thread.join();
}

unsafe extern "system" fn keyboard_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code >= 0 {
        let data = unsafe {
            &*(lparam as *const windows_sys::Win32::UI::WindowsAndMessaging::KBDLLHOOKSTRUCT)
        };
        if data.flags & LLKHF_INJECTED == 0 {
            let pressed = wparam == WM_KEYDOWN as usize || wparam == WM_SYSKEYDOWN as usize;
            let released = wparam == WM_KEYUP as usize || wparam == WM_SYSKEYUP as usize;
            if pressed || released {
                let is_leader = data.vkCode == LEADER_VK.load(Ordering::Acquire);
                let active = ACTIVE.load(Ordering::Acquire);
                if is_leader || active {
                    let duplicate_leader = is_leader
                        && if pressed {
                            LEADER_DOWN.swap(true, Ordering::AcqRel)
                        } else {
                            LEADER_DOWN.store(false, Ordering::Release);
                            false
                        };
                    if !duplicate_leader
                        && let Some(key) = key_name(data.vkCode as u16)
                        && let Some(sender) = sender_slot()
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .as_ref()
                    {
                        let _ = sender.send(KeyEvent { key, pressed });
                    }
                    return 1;
                }
            }
        }
    }
    unsafe { CallNextHookEx(null_mut(), code, wparam, lparam) }
}

unsafe extern "system" fn window_proc(
    window: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // Drawing is performed synchronously by show(); validate incidental paints.
    if message == windows_sys::Win32::UI::WindowsAndMessaging::WM_PAINT {
        let mut paint: PAINTSTRUCT = unsafe { zeroed() };
        unsafe {
            BeginPaint(window, &mut paint);
            EndPaint(window, &paint);
        }
        return 0;
    }
    unsafe { DefWindowProcW(window, message, wparam, lparam) }
}

fn virtual_screen_bounds() -> Rect {
    Rect {
        x: unsafe { GetSystemMetrics(SM_XVIRTUALSCREEN) },
        y: unsafe { GetSystemMetrics(SM_YVIRTUALSCREEN) },
        width: unsafe { GetSystemMetrics(SM_CXVIRTUALSCREEN) }.max(1) as u32,
        height: unsafe { GetSystemMetrics(SM_CYVIRTUALSCREEN) }.max(1) as u32,
    }
}

fn cursor_monitor_bounds() -> Option<Rect> {
    unsafe {
        let mut point = POINT { x: 0, y: 0 };
        if GetCursorPos(&mut point) == 0 {
            return None;
        }
        let monitor = MonitorFromPoint(point, MONITOR_DEFAULTTONEAREST);
        if monitor.is_null() {
            return None;
        }
        let mut info: MONITORINFO = zeroed();
        info.cbSize = size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return None;
        }
        Some(Rect {
            x: info.rcMonitor.left,
            y: info.rcMonitor.top,
            width: (info.rcMonitor.right - info.rcMonitor.left) as u32,
            height: (info.rcMonitor.bottom - info.rcMonitor.top) as u32,
        })
    }
}

fn focused_monitor_bounds() -> Option<Rect> {
    unsafe {
        let window = GetForegroundWindow();
        if window.is_null() {
            return None;
        }
        let monitor =
            windows_sys::Win32::Graphics::Gdi::MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
        monitor_bounds(monitor)
    }
}

unsafe fn monitor_bounds(monitor: windows_sys::Win32::Graphics::Gdi::HMONITOR) -> Option<Rect> {
    if monitor.is_null() {
        return None;
    }
    let mut info: MONITORINFO = unsafe { zeroed() };
    info.cbSize = size_of::<MONITORINFO>() as u32;
    if unsafe { GetMonitorInfoW(monitor, &mut info) } == 0 {
        return None;
    }
    Some(Rect {
        x: info.rcMonitor.left,
        y: info.rcMonitor.top,
        width: (info.rcMonitor.right - info.rcMonitor.left) as u32,
        height: (info.rcMonitor.bottom - info.rcMonitor.top) as u32,
    })
}

fn vk_for_name(name: &str) -> Option<VIRTUAL_KEY> {
    let lower = name.to_ascii_lowercase();
    match lower.as_str() {
        "capslock" => Some(VK_CAPITAL),
        "f9" => Some(VK_F9),
        "escape" => Some(VK_ESCAPE),
        "backspace" => Some(VK_BACK),
        "space" => Some(VK_SPACE),
        value if value.len() == 1 => {
            let byte = value.as_bytes()[0];
            match byte {
                b'a'..=b'z' => Some(byte.to_ascii_uppercase() as VIRTUAL_KEY),
                b'0'..=b'9' => Some(byte as VIRTUAL_KEY),
                b';' => Some(0xBA),
                b'=' => Some(0xBB),
                b',' => Some(0xBC),
                b'-' => Some(0xBD),
                b'.' => Some(0xBE),
                b'/' => Some(0xBF),
                b'`' => Some(0xC0),
                b'[' => Some(0xDB),
                b'\\' => Some(0xDC),
                b']' => Some(0xDD),
                b'\'' => Some(0xDE),
                _ => None,
            }
        }
        _ => None,
    }
}

fn key_name(vk: u16) -> Option<String> {
    match vk {
        0x08 => Some("backspace".into()),
        0x09 => Some("tab".into()),
        0x0D => Some("enter".into()),
        0x10 => Some("shift".into()),
        0x11 => Some("control".into()),
        0x12 => Some("alt".into()),
        0x14 => Some("capslock".into()),
        0x1B => Some("escape".into()),
        0x20 => Some("space".into()),
        0x21 => Some("pageup".into()),
        0x22 => Some("pagedown".into()),
        0x23 => Some("end".into()),
        0x24 => Some("home".into()),
        0x25 => Some("left".into()),
        0x26 => Some("up".into()),
        0x27 => Some("right".into()),
        0x28 => Some("down".into()),
        0x2D => Some("insert".into()),
        0x2E => Some("delete".into()),
        0x30..=0x39 | 0x41..=0x5A => {
            char::from_u32(vk as u32).map(|character| character.to_ascii_lowercase().to_string())
        }
        0x70..=0x87 => Some(format!("f{}", vk - VK_F1 + 1)),
        0xBA => Some(";".into()),
        0xBB => Some("=".into()),
        0xBC => Some(",".into()),
        0xBD => Some("-".into()),
        0xBE => Some(".".into()),
        0xBF => Some("/".into()),
        0xC0 => Some("`".into()),
        0xDB => Some("[".into()),
        0xDC => Some("\\".into()),
        0xDD => Some("]".into()),
        0xDE => Some("'".into()),
        _ => None,
    }
}

fn parse_color(value: &str, fallback: u32) -> COLORREF {
    let rgb = u32::from_str_radix(value.trim_start_matches('#'), 16).unwrap_or(fallback);
    ((rgb & 0xff) << 16) | (rgb & 0xff00) | ((rgb >> 16) & 0xff)
}

fn darken(color: COLORREF) -> COLORREF {
    ((color & 0xff) / 2) | ((((color >> 8) & 0xff) / 2) << 8) | ((((color >> 16) & 0xff) / 2) << 16)
}

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}

fn wide_without_nul(value: &str) -> Vec<u16> {
    value.encode_utf16().collect()
}

fn last_error(message: &str) -> anyhow::Error {
    let error = unsafe { GetLastError() };
    anyhow!("{message} (Win32 error {error})")
}
