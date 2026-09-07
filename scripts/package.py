#!/usr/bin/env python3
"""Package a built kbmouse release. Uses only Python's standard library and OS tools."""
import argparse
import os
from pathlib import Path
import plistlib
import shutil
import subprocess
import tarfile
import tempfile
import re
import zipfile

ROOT = Path(__file__).resolve().parents[1]
TARGETS = ("aarch64-apple-darwin", "x86_64-apple-darwin", "x86_64-pc-windows-msvc", "x86_64-unknown-linux-gnu")


def run(*args):
    subprocess.run([str(arg) for arg in args], check=True)


def version():
    text = (ROOT / "Cargo.toml").read_text()
    package = text.split("[package]", 1)[1].split("\n[", 1)[0]
    return re.search(r'^version\s*=\s*"([^"]+)"', package, re.MULTILINE).group(1)


def mac_bundle(stage, binary, release, identity):
    app = stage / "kbmouse.app"
    contents = app / "Contents"
    (contents / "MacOS").mkdir(parents=True)
    resources = contents / "Resources"
    resources.mkdir()
    shutil.copy2(binary, contents / "MacOS/kbmouse")
    (contents / "MacOS/kbmouse").chmod(0o755)
    info = {
        "CFBundleIdentifier": "com.ryclic.kbmouse",
        "CFBundleName": "kbmouse",
        "CFBundleDisplayName": "kbmouse",
        "CFBundleExecutable": "kbmouse",
        "CFBundlePackageType": "APPL",
        "CFBundleShortVersionString": release,
        "CFBundleVersion": release,
        "CFBundleIconFile": "kbmouse.icns",
        "LSMinimumSystemVersion": "11.0",
        "NSHighResolutionCapable": True,
    }
    with (contents / "Info.plist").open("wb") as dest:
        plistlib.dump(info, dest)
    iconset = stage / "kbmouse.iconset"
    iconset.mkdir()
    for size in (16, 32, 128, 256, 512):
        for scale in (1, 2):
            suffix = "@2x" if scale == 2 else ""
            run("sips", "-z", size * scale, size * scale, ROOT / "assets/logo.png",
                "--out", iconset / f"icon_{size}x{size}{suffix}.png")
    run("iconutil", "-c", "icns", iconset, "-o", resources / "kbmouse.icns")
    args = ["codesign", "--force", "--sign", identity]
    if identity != "-":
        args += ["--options", "runtime", "--timestamp"]
    run(*args, app)
    run("codesign", "--verify", "--deep", "--strict", app)
    return app


def package(target, binary, out, installer=False, identity="-", notary_profile=None):
    if target not in TARGETS:
        raise ValueError(f"Unsupported release target: {target}")
    if not binary.is_file():
        raise FileNotFoundError(f"Build the release executable first: {binary}")
    release = version()
    out.mkdir(parents=True, exist_ok=True)
    name = f"kbmouse-{release}-{target}"
    outputs = []
    with tempfile.TemporaryDirectory(prefix="kbmouse-package-") as temp:
        stage = Path(temp)
        if target.endswith("apple-darwin"):
            app = mac_bundle(stage, binary, release, identity)
            if notary_profile:
                if identity == "-":
                    raise ValueError("Notarization requires a Developer ID signing identity")
                submission = stage / "notarize.zip"
                run("ditto", "-c", "-k", "--sequesterRsrc", "--keepParent", app, submission)
                run("xcrun", "notarytool", "submit", submission, "--keychain-profile", notary_profile, "--wait")
                run("xcrun", "stapler", "staple", app)
            archive = out / f"{name}.tar.gz"
            with tarfile.open(archive, "w:gz", dereference=False) as tar:
                tar.add(app, arcname="kbmouse.app")
            outputs.append(archive)
            if installer:
                # DMG contents contain only the bundle and the standard Applications shortcut.
                dmg_root = stage / "dmg"
                dmg_root.mkdir()
                run("ditto", app, dmg_root / "kbmouse.app")
                (dmg_root / "Applications").symlink_to("/Applications")
                dmg = out / f"{name}.dmg"
                run("hdiutil", "create", "-ov", "-volname", "kbmouse", "-srcfolder", dmg_root, "-format", "UDZO", dmg)
                if identity != "-":
                    run("codesign", "--force", "--sign", identity, "--timestamp", dmg)
                if notary_profile:
                    run("xcrun", "notarytool", "submit", dmg, "--keychain-profile", notary_profile, "--wait")
                    run("xcrun", "stapler", "staple", dmg)
                outputs.append(dmg)
        elif target.endswith("windows-msvc"):
            archive = out / f"{name}.zip"
            with zipfile.ZipFile(archive, "w", zipfile.ZIP_DEFLATED) as zipped:
                zipped.write(binary, "kbmouse.exe")
                zipped.write(ROOT / "README.md", "README.md")
            outputs.append(archive)
            if installer:
                compiler = shutil.which("iscc") or str(Path(os.environ.get("ProgramFiles(x86)", r"C:\Program Files (x86)")) / "Inno Setup 6/ISCC.exe")
                run(compiler, f"/DAppVersion={release}", f"/DBinaryPath={binary.resolve()}",
                    f"/O{out.resolve()}", f"/F{name}-setup", ROOT / "packaging/windows/kbmouse.iss")
                outputs.append(out / f"{name}-setup.exe")
        else:
            archive = out / f"{name}.tar.gz"
            with tarfile.open(archive, "w:gz") as tar:
                def executable(info):
                    if info.name in ("kbmouse", "install.sh", "uninstall.sh"):
                        info.mode = 0o755
                    return info
                tar.add(binary, "kbmouse", filter=executable)
                tar.add(ROOT / "assets/logo.png", "logo.png")
                tar.add(ROOT / "README.md", "README.md")
                for script in ("install.sh", "uninstall.sh"):
                    tar.add(ROOT / "packaging/linux" / script, script, filter=executable)
            outputs.append(archive)
    return outputs


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--binary", type=Path)
    parser.add_argument("--out", type=Path, default=ROOT / "dist")
    parser.add_argument("--installer", action="store_true")
    parser.add_argument("--sign-identity", default="-", help="macOS Developer ID identity; '-' makes a local ad-hoc build")
    parser.add_argument("--notary-profile", help="Keychain profile created with xcrun notarytool store-credentials")
    args = parser.parse_args()
    binary = args.binary or ROOT / "target" / args.target / "release" / ("kbmouse.exe" if "windows" in args.target else "kbmouse")
    for output in package(args.target, binary, args.out, args.installer, args.sign_identity, args.notary_profile):
        print(output)


if __name__ == "__main__":
    main()
