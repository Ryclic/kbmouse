use anyhow::{Result, bail};

#[cfg(target_os = "linux")]
use anyhow::Context;

#[cfg(target_os = "linux")]
pub struct SingleInstance {
    _listener: std::os::unix::net::UnixListener,
    path: std::path::PathBuf,
}

#[cfg(target_os = "linux")]
impl SingleInstance {
    pub fn acquire() -> Result<Self> {
        use std::os::unix::net::{UnixListener, UnixStream};

        let user = std::env::var("USER").unwrap_or_else(|_| "user".into());
        let path = std::env::temp_dir().join(format!("kbmouse-{user}.sock"));
        if path.exists() {
            if UnixStream::connect(&path).is_ok() {
                bail!("another kbmouse instance is already running");
            }
            std::fs::remove_file(&path).context("failed to remove stale instance socket")?;
        }
        let listener = UnixListener::bind(&path).context("failed to create instance socket")?;
        Ok(Self {
            _listener: listener,
            path,
        })
    }
}

#[cfg(target_os = "linux")]
impl Drop for SingleInstance {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(windows)]
pub struct SingleInstance(windows_sys::Win32::Foundation::HANDLE);

#[cfg(windows)]
impl SingleInstance {
    pub fn acquire() -> Result<Self> {
        use windows_sys::Win32::{
            Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError},
            System::Threading::CreateMutexW,
        };

        let name: Vec<u16> = "Local\\kbmouse-single-instance\0".encode_utf16().collect();
        let handle = unsafe { CreateMutexW(std::ptr::null(), 0, name.as_ptr()) };
        if handle.is_null() {
            bail!("failed to create single-instance mutex");
        }
        if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
            unsafe { CloseHandle(handle) };
            bail!("another kbmouse instance is already running");
        }
        Ok(Self(handle))
    }
}

#[cfg(windows)]
impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}
