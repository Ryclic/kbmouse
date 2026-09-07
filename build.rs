use std::{env, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=src/platform/macos/native.m");
    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("macos") {
        return;
    }
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    let arch = match env::var("CARGO_CFG_TARGET_ARCH").unwrap().as_str() {
        "aarch64" => "arm64",
        "x86_64" => "x86_64",
        other => panic!("unsupported macOS architecture: {other}"),
    };
    let status = Command::new("xcrun")
        .args([
            "clang",
            "-arch",
            arch,
            "-fobjc-arc",
            "-fblocks",
            "-Wall",
            "-Wextra",
            "-Werror",
            "-mmacosx-version-min=11.0",
            "-c",
            "src/platform/macos/native.m",
            "-o",
        ])
        .arg(out.join("native.o"))
        .status()
        .expect("macOS builds require Xcode Command Line Tools (xcode-select --install)");
    assert!(status.success(), "failed to compile macOS backend");
    assert!(
        Command::new("ar")
            .arg("crs")
            .arg(out.join("libkbmouse_native.a"))
            .arg(out.join("native.o"))
            .status()
            .unwrap()
            .success()
    );
    println!("cargo:rustc-link-search=native={}", out.display());
    println!("cargo:rustc-link-lib=static=kbmouse_native");
    for framework in ["AppKit", "ApplicationServices", "IOKit"] {
        println!("cargo:rustc-link-lib=framework={framework}");
    }
}
