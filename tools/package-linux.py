#!/usr/bin/env python3
"""Build Linux packages locally using the Tauri bundler; never publish them."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def run(*command, cwd=ROOT):
    subprocess.run([str(arg) for arg in command], cwd=cwd, check=True)


def notices(destination):
    """Ship the license texts supplied by dependencies, alongside a package index."""
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT, text=True))
    destination.mkdir(parents=True, exist_ok=True)
    rows = []
    for package in sorted(metadata["packages"], key=lambda p: (p["name"], p["version"])):
        rows.append(f'{package["name"]} {package["version"]}: {package.get("license") or "see source"}')
        source = Path(package["manifest_path"]).parent
        texts = [p for p in source.iterdir() if p.is_file()
                 and p.name.lower().startswith(("license", "copying", "notice"))]
        if not texts:
            # Git workspace crates may keep their license at the checkout root.
            texts = [p for p in source.parent.iterdir() if p.is_file()
                     and p.name.lower().startswith(("license", "copying", "notice"))]
        if texts:
            target = destination / f'{package["name"]}-{package["version"]}'
            target.mkdir(exist_ok=True)
            for text in texts:
                shutil.copyfile(text, target / text.name)
    # Fonts and frontend dependencies are separate from Cargo's dependency graph.
    modules = ROOT / "apps/macafon-ui/node_modules"
    frontend = json.loads((ROOT / "apps/macafon-ui/package.json").read_text())
    for name in [*frontend["dependencies"], "svelte"]:
        package = modules / name
        if not package.is_dir():
            continue
        info = json.loads((package / "package.json").read_text())
        rows.append(f'{name} {info["version"]}: {info.get("license", "see source")}')
        for text in package.iterdir():
            if text.is_file() and text.name.lower().startswith(("license", "ofl", "notice")):
                target = destination / name.replace("/", "__")
                target.mkdir(exist_ok=True)
                shutil.copyfile(text, target / text.name)
    (destination / "INDEX.txt").write_text("\n".join(rows) + "\n")


def configuration(stage, triple):
    sidecars = []
    for name in ("macafond", "macafon"):
        shutil.copy2(ROOT / "target/release" / name, stage / f"{name}-{triple}")
        sidecars.append(str(stage / name))
    files = {
        "/usr/lib/systemd/user/macafond.service": str(ROOT / "packaging/systemd/macafond.service"),
        "/usr/share/macafon/cleanup-user-sessions.sh": str(ROOT / "packaging/linux/cleanup-user-sessions.sh"),
        "/usr/share/doc/macafon/LICENSE": str(ROOT / "LICENSE"),
    }
    for icon in (ROOT / "packaging/icons/hicolor").rglob("*"):
        if icon.is_file():
            files[f'/usr/share/icons/hicolor/{icon.relative_to(ROOT / "packaging/icons/hicolor")}'] = str(icon)
    common = {
        "files": files,
        "desktopTemplate": str(ROOT / "packaging/linux/desktop.hbs"),
        "preRemoveScript": str(ROOT / "packaging/linux/prerm.sh"),
    }
    return {
        "bundle": {
            "active": True,
            "targets": ["deb", "rpm", "appimage"],
            "publisher": "Mar1kLab",
            "homepage": "https://github.com/Mar1kLab/Macafon",
            "category": "Music",
            "license": "GPL-3.0-only",
            "licenseFile": str(ROOT / "LICENSE"),
            "shortDescription": "Microphone processing with a virtual microphone",
            "externalBin": sidecars,
            "resources": {str(stage / "licenses"): "licenses"},
            "linux": {
                "deb": {**common, "section": "sound", "depends": [
                    "pipewire (>= 1.0)", "wireplumber (>= 0.5)", "libwebkit2gtk-4.1-0", "libgtk-3-0"]},
                "rpm": {**common, "depends": ["pipewire >= 1.0", "wireplumber >= 0.5", "webkit2gtk4.1", "gtk3"]},
                "appimage": {"bundleMediaFramework": False},
            },
        }
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundles", default="deb,rpm,appimage")
    parser.add_argument("--bundle-only", action="store_true", help="use existing release binaries")
    parser.add_argument("--build-only", action="store_true", help="build release binaries without packages")
    args = parser.parse_args()
    if os.name != "posix" or not shutil.which("pnpm"):
        parser.error("Linux and pnpm are required")
    if args.bundle_only and args.build_only:
        parser.error("choose either --bundle-only or --build-only")
    triple = next(line.split(": ", 1)[1] for line in subprocess.check_output(
        ["rustc", "-vV"], text=True).splitlines() if line.startswith("host:"))
    stage = ROOT / "target/package-linux"
    stage.mkdir(parents=True, exist_ok=True)
    if not args.bundle_only:
        run("cargo", "build", "--release", "--locked", "-p", "macafond", "-p", "macafon-cli")
    notices(stage / "licenses")
    config = stage / "tauri-bundle.conf.json"
    config.write_text(json.dumps(configuration(stage, triple), indent=2) + "\n")
    ui = ROOT / "apps/macafon-ui"
    if not args.bundle_only:
        run("pnpm", "tauri", "build", "--no-bundle", "--config", config, cwd=ui)
    if not args.build_only:
        run("pnpm", "tauri", "bundle", "--bundles", args.bundles, "--config", config, cwd=ui)
        version = tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"]
        for package in (ROOT / "target/release/bundle/deb").glob("*.deb"):
            recipe = ROOT / "target/package-linux/aur/macafon-bin"
            recipe.mkdir(parents=True, exist_ok=True)
            checksum = hashlib.sha256(package.read_bytes()).hexdigest()
            shutil.copy2(ROOT / "packaging/aur/macafon-bin/macafon.install", recipe / "macafon.install")
            (recipe / "PKGBUILD").write_text(f'''# Generated from the release artifact; publish alongside that exact file.
pkgname=macafon-bin
pkgver={version}
pkgrel=1
pkgdesc='Microphone processing with a virtual microphone'
arch=('x86_64')
url='https://github.com/Mar1kLab/Macafon'
license=('GPL-3.0-only')
depends=('pipewire>=1.0' 'wireplumber>=0.5' 'webkit2gtk-4.1' 'gtk3' 'libsoup3')
makedepends=('libarchive')
provides=('macafon')
conflicts=('macafon')
install=macafon.install
source=("$url/releases/download/v$pkgver/{package.name}" 'macafon.install')
sha256sums=('{checksum}' '{hashlib.sha256((recipe / 'macafon.install').read_bytes()).hexdigest()}')
package() {{
    bsdtar -xf "$srcdir/{package.name}" -C "$srcdir"
    bsdtar -xf "$srcdir"/data.tar.* -C "$pkgdir"
}}
''')
        print(f"Macafon {version}: packages are in target/release/bundle/")


if __name__ == "__main__":
    main()
