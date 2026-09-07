//! Signed GitHub release updates for direct, user-writable installations.
use anyhow::{Context, Result, bail, ensure};
use self_update::backends::github::Update;
use semver::Version;
use std::{
    path::{Path, PathBuf},
    process::Command,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

pub const RELEASES_URL: &str = "https://github.com/Ryclic/kbmouse/releases";
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const PUBLIC_KEY: Option<&str> = option_env!("KBMOUSE_UPDATE_PUBLIC_KEY");

fn decode_key(value: &str) -> Result<[u8; 32]> {
    let value = value.trim();
    ensure!(
        value.len() == 64 && value.is_ascii(),
        "update public key must be 64 hexadecimal characters"
    );
    let mut key = [0; 32];
    for (i, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16)
            .context("invalid update public key")?;
    }
    ensure!(key != [0; 32], "update public key cannot be zero");
    Ok(key)
}
fn public_key() -> Result<[u8; 32]> {
    decode_key(PUBLIC_KEY.context("Updates are unavailable in this build. Install an official release to enable signed updates.")?)
}
fn archive_name(version: &str, target: &str) -> Result<String> {
    Version::parse(version).context("invalid release version")?;
    let extension = match target {
        "aarch64-apple-darwin" | "x86_64-apple-darwin" | "x86_64-unknown-linux-gnu" => "tar.gz",
        "x86_64-pc-windows-msvc" => "zip",
        _ => bail!("Automatic updates are not published for {target}"),
    };
    Ok(format!("kbmouse-{version}-{target}.{extension}"))
}
fn newer(version: &str, current: &str) -> Result<bool> {
    let version = Version::parse(version)?;
    Ok(version.pre.is_empty() && version > Version::parse(current)?)
}
fn app_bundle(exe: &Path) -> Option<PathBuf> {
    let macos = exe.parent()?;
    let contents = macos.parent()?;
    let app = contents.parent()?;
    (macos.file_name()? == "MacOS"
        && contents.file_name()? == "Contents"
        && app.extension()? == "app")
        .then(|| app.to_path_buf())
}
fn installation_supported(exe: &Path) -> Result<()> {
    if cfg!(target_os = "macos") {
        ensure!(
            app_bundle(exe).is_some(),
            "Install kbmouse.app from the DMG before using automatic updates."
        );
    }
    if cfg!(target_os = "linux") {
        ensure!(
            !["/usr", "/bin", "/sbin", "/opt", "/nix", "/snap", "/app"]
                .iter()
                .any(|root| exe.starts_with(root)),
            "This installation is managed outside kbmouse. Update it with its package manager, or use the per-user installer."
        );
    }
    Ok(())
}
fn builder(exe: &Path) -> Result<self_update::backends::github::UpdateBuilder> {
    installation_supported(exe)?;
    let key = public_key()?;
    let target = self_update::get_target();
    archive_name(CURRENT_VERSION, target)?;
    let mut builder = Update::configure();
    builder
        .repo_owner("Ryclic")
        .repo_name("kbmouse")
        .bin_name("kbmouse")
        .target(target)
        .current_version(CURRENT_VERSION)
        .no_confirm(true)
        .show_output(false)
        .show_download_progress(false)
        .timeout(Duration::from_secs(60))
        .check_install_path_writable(true)
        .verifying_keys([key]);
    if cfg!(target_os = "macos") {
        builder
            .bundle_path_in_archive("kbmouse.app")
            .bundle_install_path(app_bundle(exe).context("missing app bundle")?);
        builder.verify_binary(|app| {
            let status = Command::new("/usr/bin/codesign")
                .args([
                    "--verify",
                    "--deep",
                    "--strict",
                    "-R",
                    "identifier \"com.ryclic.kbmouse\"",
                ])
                .arg(app)
                .status()?;
            if !status.success() {
                return Err(self_update::Error::verification_rejected(
                    "The new app's code signature is invalid",
                ));
            }
            Ok(())
        });
    } else {
        builder.bin_install_path(exe);
    }
    Ok(builder)
}

pub fn check(exe: &Path) -> Result<Option<String>> {
    let update = builder(exe)?.build()?;
    let Some(release) = update.is_update_available()? else {
        return Ok(None);
    };
    if !newer(release.version(), CURRENT_VERSION)? {
        return Ok(None);
    }
    let name = archive_name(release.version(), self_update::get_target())?;
    ensure!(
        release.assets().iter().any(|asset| asset.name() == name),
        "This release does not yet have a download for your platform."
    );
    Ok(Some(release.version().to_owned()))
}
pub fn install(
    exe: &Path,
    version: &str,
    progress: impl Fn(u64, Option<u64>) + Send + Sync + 'static,
) -> Result<bool> {
    ensure!(
        newer(version, CURRENT_VERSION)?,
        "Refusing to install an older or prerelease version"
    );
    let name = archive_name(version, self_update::get_target())?;
    let mut update = builder(exe)?;
    // Pin exactly the version the user selected. Never use fuzzy platform matching,
    // which could select a DMG/installer or an archive for the wrong architecture.
    update
        .release_tag(format!("v{version}"))
        .asset_matcher(move |assets| assets.iter().find(|asset| asset.name() == name).cloned())
        .progress_callback(progress);
    Ok(update.build()?.update()?.is_updated())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Idle,
    Disabled(String),
    Checking,
    Current,
    Available(String),
    Installing,
    Installed(String),
    Error(String),
}
enum Event {
    Checked(std::result::Result<Option<String>, String>),
    Installed(std::result::Result<bool, String>, String),
    Progress(u64, Option<u64>),
}
struct Worker {
    handle: JoinHandle<()>,
    installing: bool,
}
pub struct Updater {
    pub status: Status,
    pub progress: (u64, Option<u64>),
    executable: PathBuf,
    sender: crossbeam_channel::Sender<Event>,
    receiver: crossbeam_channel::Receiver<Event>,
    worker: Option<Worker>,
    next_check: Instant,
}
impl Updater {
    pub fn new(executable: PathBuf) -> Self {
        let (sender, receiver) = crossbeam_channel::unbounded();
        let status = match public_key().and_then(|_| installation_supported(&executable)) {
            Ok(()) => Status::Idle,
            Err(error) => Status::Disabled(error.to_string()),
        };
        Self {
            status,
            progress: (0, None),
            executable,
            sender,
            receiver,
            worker: None,
            next_check: Instant::now(),
        }
    }
    pub fn busy(&self) -> bool {
        self.worker.is_some()
    }
    pub fn installing(&self) -> bool {
        self.worker.as_ref().is_some_and(|worker| worker.installing)
    }
    pub fn can_check(&self) -> bool {
        !self.busy() && !matches!(self.status, Status::Disabled(_) | Status::Installed(_))
    }
    pub fn tick(&mut self, automatic: bool) {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                Event::Progress(done, total) => self.progress = (done, total),
                Event::Checked(result) => {
                    self.finish_worker();
                    self.status = match result {
                        Ok(Some(v)) => Status::Available(v),
                        Ok(None) => Status::Current,
                        Err(e) => Status::Error(e),
                    };
                }
                Event::Installed(result, version) => {
                    self.finish_worker();
                    self.status = match result {
                        Ok(true) => Status::Installed(version),
                        Ok(false) => Status::Current,
                        Err(e) => Status::Error(e),
                    };
                }
            }
        }
        if automatic && self.can_check() && Instant::now() >= self.next_check {
            self.check();
        }
    }
    fn finish_worker(&mut self) {
        if let Some(worker) = self.worker.take() {
            let _ = worker.handle.join();
        }
        self.next_check = Instant::now() + CHECK_INTERVAL;
    }
    pub fn check(&mut self) {
        if !self.can_check() {
            return;
        }
        self.status = Status::Checking;
        let exe = self.executable.clone();
        let sender = self.sender.clone();
        self.spawn(false, move || {
            let _ = sender.send(Event::Checked(check(&exe).map_err(|e| format!("{e:#}"))));
        });
    }
    pub fn install(&mut self, version: String) {
        if self.busy() || self.status != Status::Available(version.clone()) {
            return;
        }
        self.status = Status::Installing;
        self.progress = (0, None);
        let exe = self.executable.clone();
        let sender = self.sender.clone();
        self.spawn(true, move || {
            let updates = sender.clone();
            let result = install(&exe, &version, move |done, total| {
                let _ = updates.send(Event::Progress(done, total));
            });
            let _ = sender.send(Event::Installed(
                result.map_err(|e| format!("{e:#}")),
                version,
            ));
        });
    }
    fn spawn(&mut self, installing: bool, work: impl FnOnce() + Send + 'static) {
        match thread::Builder::new()
            .name("kbmouse-updater".into())
            .spawn(work)
        {
            Ok(handle) => self.worker = Some(Worker { handle, installing }),
            Err(error) => {
                self.status = Status::Error(error.to_string());
                self.next_check = Instant::now() + CHECK_INTERVAL;
            }
        }
    }
}
impl Drop for Updater {
    fn drop(&mut self) {
        // Never let a normal app shutdown interrupt an in-progress installation.
        // Read-only checks can be abandoned when the process exits.
        if let Some(worker) = self.worker.take()
            && worker.installing
        {
            let _ = worker.handle.join();
        }
    }
}

/// Called only after joining the input runtime and dropping the single-instance lock.
/// Keep the original path: after self-replacement current_exe may identify the old image.
pub fn restart(executable: &Path) -> Result<()> {
    let mut command = Command::new(executable);
    command.args(std::env::args_os().skip(1));
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        Err(command.exec()).context("Could not restart kbmouse; launch the installed app again")
    }
    #[cfg(windows)]
    {
        command
            .spawn()
            .context("Could not restart kbmouse; launch the installed app again")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_keys_versions_and_exact_asset_names() {
        assert!(decode_key("").is_err());
        assert!(decode_key(&"00".repeat(32)).is_err());
        assert!(decode_key(&"gg".repeat(32)).is_err());
        assert_eq!(decode_key(&"12".repeat(32)).unwrap(), [0x12; 32]);
        assert!(newer("0.2.0", "0.1.0").unwrap());
        assert!(!newer("0.1.0", "0.1.0").unwrap());
        assert!(!newer("0.1.0", "0.2.0").unwrap());
        assert!(!newer("0.2.0-beta.1", "0.1.0").unwrap());
        assert_eq!(
            archive_name("0.2.0", "x86_64-pc-windows-msvc").unwrap(),
            "kbmouse-0.2.0-x86_64-pc-windows-msvc.zip"
        );
        assert!(archive_name("0.2.0", "aarch64-unknown-linux-gnu").is_err());
    }
    #[test]
    fn finds_only_complete_app_bundles() {
        assert_eq!(
            app_bundle(Path::new(
                "/Applications/kbmouse.app/Contents/MacOS/kbmouse"
            )),
            Some(PathBuf::from("/Applications/kbmouse.app"))
        );
        assert!(app_bundle(Path::new("/tmp/kbmouse")).is_none());
        assert!(app_bundle(Path::new("/tmp/fake.app/kbmouse")).is_none());
    }
    #[test]
    fn verifies_signatures_and_rejects_unsigned_tampered_or_renamed_archives() {
        use std::io::{Seek, SeekFrom, Write};
        let temp = tempfile::tempdir().unwrap();
        let raw = temp.path().join("unsigned.tar.gz");
        let signed = temp
            .path()
            .join("kbmouse-0.2.0-x86_64-unknown-linux-gnu.tar.gz");
        let file = std::fs::File::create(&raw).unwrap();
        let gzip = flate2::write::GzEncoder::new(file, flate2::Compression::default());
        let mut tar = tar::Builder::new(gzip);
        let mut header = tar::Header::new_gnu();
        header.set_size(4);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "kbmouse", &b"test"[..])
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap();
        let key = zipsign_api::SigningKey::from_bytes(&[7; 32]);
        let verifying = *key.verifying_key().as_bytes();
        let mut input = std::fs::File::open(&raw).unwrap();
        let mut output = std::fs::File::create(&signed).unwrap();
        zipsign_api::sign::copy_and_sign_tar(
            &mut input,
            &mut output,
            &[key],
            Some(signed.file_name().unwrap().to_str().unwrap().as_bytes()),
        )
        .unwrap();
        drop(output);
        assert!(self_update::update::verify_signature(&signed, &[verifying]).is_ok());
        assert!(self_update::update::verify_signature(&raw, &[verifying]).is_err());
        assert!(self_update::update::verify_signature(&signed, &[[9; 32]]).is_err());
        let renamed = temp.path().join("other.tar.gz");
        std::fs::copy(&signed, &renamed).unwrap();
        assert!(self_update::update::verify_signature(&renamed, &[verifying]).is_err());
        let mut changed = std::fs::OpenOptions::new()
            .write(true)
            .open(&signed)
            .unwrap();
        changed.seek(SeekFrom::Start(20)).unwrap();
        changed.write_all(b"tampered").unwrap();
        drop(changed);
        assert!(self_update::update::verify_signature(&signed, &[verifying]).is_err());
    }
}
