use crate::{
    config::Config,
    engine::{MouseButton, Scene},
    geometry::Rect,
};
use anyhow::Result;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct KeyEvent {
    pub key: String,
    pub pressed: bool,
}

pub trait Backend {
    fn screen_bounds(&self) -> Rect;
    fn next_event(&mut self, timeout: Duration) -> Result<Option<KeyEvent>>;
    fn apply_config(&mut self, config: &Config) -> Result<()>;
    fn set_active(&mut self, active: bool);
    fn show(&mut self, scene: &Scene) -> Result<()>;
    fn hide(&mut self) -> Result<()>;
    fn move_to(&mut self, x: i32, y: i32) -> Result<()>;
    fn move_by(&mut self, dx: i32, dy: i32) -> Result<()>;
    fn button(&mut self, button: MouseButton, down: bool) -> Result<()>;
    fn scroll(&mut self, amount: i32) -> Result<()>;
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::NativeBackend;

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::NativeBackend;

#[cfg(not(any(windows, target_os = "linux")))]
compile_error!("kbmouse currently supports Windows and Linux only");
