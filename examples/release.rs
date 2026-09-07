//! Release-only key generation and archive signing. Never linked into the app.
use anyhow::{Context, Result, bail, ensure};
use clap::{Parser, Subcommand};
use std::{fs::File, io::Write, path::PathBuf};

#[derive(Parser)]
struct Args {
    #[command(subcommand)]
    command: Action,
}
#[derive(Subcommand)]
enum Action {
    /// Write a new private seed to a file and print its public key.
    Keygen { private_key: PathBuf },
    /// Sign update archives in place using environment-provided keys.
    Sign { archives: Vec<PathBuf> },
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
fn seed(value: &str) -> Result<[u8; 32]> {
    let value = value.trim();
    ensure!(
        value.is_ascii() && value.len() == 64,
        "Signing seed must be 64 hexadecimal characters"
    );
    let mut bytes = [0; 32];
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16).context("Invalid signing seed")?;
    }
    Ok(bytes)
}
fn main() -> Result<()> {
    match Args::parse().command {
        Action::Keygen { private_key } => {
            let mut bytes = [0; 32];
            getrandom::fill(&mut bytes).context("Could not generate a signing key")?;
            let key = zipsign_api::SigningKey::from_bytes(&bytes);
            let mut options = File::options();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            let mut file = options
                .open(&private_key)
                .context("Choose a new private-key file in an existing private directory")?;
            writeln!(file, "{}", hex(&bytes))?;
            file.sync_all()?;
            println!("{}", hex(key.verifying_key().as_bytes()));
            eprintln!(
                "Private key saved to {}. Keep it backed up and secret.",
                private_key.display()
            );
        }
        Action::Sign { archives } => {
            ensure!(!archives.is_empty(), "Provide at least one update archive");
            let key = zipsign_api::SigningKey::from_bytes(&seed(
                &std::env::var("KBMOUSE_UPDATE_SIGNING_KEY")
                    .context("Set KBMOUSE_UPDATE_SIGNING_KEY to the private hexadecimal seed")?,
            )?);
            let public = std::env::var("KBMOUSE_UPDATE_PUBLIC_KEY")
                .context("Set KBMOUSE_UPDATE_PUBLIC_KEY to the public key embedded in the app")?;
            ensure!(
                public
                    .trim()
                    .eq_ignore_ascii_case(&hex(key.verifying_key().as_bytes())),
                "Signing key does not match the embedded public key"
            );
            for archive in archives {
                let name = archive
                    .file_name()
                    .and_then(|n| n.to_str())
                    .context("Invalid archive filename")?;
                let mut input = File::open(&archive)?;
                let mut output = tempfile::NamedTempFile::new_in(
                    archive.parent().unwrap_or(std::path::Path::new(".")),
                )?;
                if name.ends_with(".tar.gz") {
                    zipsign_api::sign::copy_and_sign_tar(
                        &mut input,
                        output.as_file_mut(),
                        std::slice::from_ref(&key),
                        Some(name.as_bytes()),
                    )?;
                } else if name.ends_with(".zip") {
                    zipsign_api::sign::copy_and_sign_zip(
                        &mut input,
                        output.as_file_mut(),
                        std::slice::from_ref(&key),
                        Some(name.as_bytes()),
                    )?;
                } else {
                    bail!("Only .tar.gz and .zip updater archives can be signed");
                }
                output.as_file().sync_all()?;
                drop(input); // Windows cannot replace an open file.
                output.persist(&archive)?;
                self_update::update::verify_signature(
                    &archive,
                    &[*key.verifying_key().as_bytes()],
                )?;
                println!("Signed and verified {}", archive.display());
            }
        }
    }
    Ok(())
}
