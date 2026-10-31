#!/usr/bin/env python3
"""Build Linux packages locally using the Tauri bundler; never publish them."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tomllib

ROOT = Path(__file__).resolve().parents[1]


def run(*command, cwd=ROOT, env=None):
    subprocess.run([str(arg) for arg in command], cwd=cwd, env=env, check=True)


def build_environment():
    # whisper-rs-sys 0.14.1's bindgen misreads layouts with libclang 23.
    # Linux uses CPU-only Whisper; its bundled bindings match this configuration.
    # Keep this local to Linux build subprocesses: Windows needs generated Vulkan symbols.
    environment = os.environ.copy()
    environment["WHISPER_DONT_GENERATE_BINDINGS"] = "1"
    return environment


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


def stt_notices(destination):
    """Native runtimes are not separate Cargo packages; include their notices too."""
    destination.mkdir(parents=True, exist_ok=True)
    metadata = json.loads(subprocess.check_output(
        ["cargo", "metadata", "--locked", "--format-version", "1"], cwd=ROOT, text=True))
    whisper_sys = next(p for p in metadata["packages"] if p["name"] == "whisper-rs-sys")
    source = Path(whisper_sys["manifest_path"]).parent / "whisper.cpp/LICENSE"
    shutil.copyfile(source, destination / "whisper.cpp-LICENSE.txt")
    # https://github.com/microsoft/onnxruntime/blob/main/LICENSE (MIT).
    (destination / "onnxruntime-LICENSE.txt").write_text("""MIT License

Copyright (c) Microsoft Corporation

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:
The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.
THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
""", encoding="utf-8")
    (destination / "dictation-models.txt").write_text("""Models are downloaded separately, never bundled.
Whisper: OpenAI, MIT; ggml conversion/quantization by whisper.cpp contributors.
https://huggingface.co/ggerganov/whisper.cpp
Silero VAD: Silero Team, MIT; GGML conversion by whisper.cpp contributors.
https://github.com/snakers4/silero-vad
Parakeet TDT 0.6B v3: NVIDIA, CC-BY-4.0.
https://huggingface.co/nvidia/parakeet-tdt-0.6b-v3
https://creativecommons.org/licenses/by/4.0/
ONNX export and int8 quantization by the sherpa-onnx contributors:
https://huggingface.co/csukuangfj/sherpa-onnx-nemo-parakeet-tdt-0.6b-v3-int8
""", encoding="utf-8")


def stage_worker(stage):
    """Stage a relocatable worker and its shared inference runtimes, without host rpaths."""
    if not shutil.which("patchelf"):
        raise RuntimeError("patchelf is required to package the Linux dictation worker")
    runtime = stage / "runtime"
    if runtime.exists():
        shutil.rmtree(runtime)
    binaries = runtime / "usr/bin"
    libraries = runtime / "usr/lib/macafon"
    binaries.mkdir(parents=True)
    libraries.mkdir(parents=True)
    worker = binaries / "macafon-stt"
    shutil.copy2(ROOT / "target/release/macafon-stt", worker)
    # sherpa-onnx-sys copies its runtime libraries (including SONAME aliases) here.
    sources = [p for p in (ROOT / "target/release").glob("*.so*")
               if p.name.startswith(("libsherpa-onnx", "libonnxruntime"))]
    if not any(p.name.startswith("libsherpa-onnx-c-api.so") for p in sources):
        raise RuntimeError("missing sherpa-onnx shared libraries; rebuild macafon-stt-worker")
    for source in sources:
        target = libraries / source.name
        shutil.copy2(source, target)
        run("patchelf", "--set-rpath", "$ORIGIN", target)
        # Some upstream archives omit a versioned symlink required by DT_NEEDED.
        soname = subprocess.check_output(["patchelf", "--print-soname", target], text=True).strip()
        if soname and soname != target.name:
            if Path(soname).name != soname:
                raise RuntimeError(f"invalid runtime SONAME: {soname}")
            shutil.copy2(target, libraries / soname)
    # tauri-build also copies this sidecar into target/release; the second entry
    # lets that development copy find sherpa-onnx-sys's adjacent runtime files.
    run("patchelf", "--set-rpath", "$ORIGIN/../lib/macafon:$ORIGIN", worker)
    # Resolve with the private directory, not a developer's LD_LIBRARY_PATH.
    environment = {key: value for key, value in os.environ.items() if key != "LD_LIBRARY_PATH"}
    for binary in (worker, *libraries.iterdir()):
        result = subprocess.run(["ldd", str(binary)], env=environment, text=True,
                                stdout=subprocess.PIPE, stderr=subprocess.STDOUT, check=True)
        if "not found" in result.stdout:
            raise RuntimeError(f"unresolved packaged runtime dependencies for {binary}:\n{result.stdout}")
    return worker, libraries


def aur_optdepends():
    return (ROOT / "packaging/aur/optdepends.sh").read_text().strip()


def configuration(stage, triple):
    sidecars = []
    worker, libraries = stage_worker(stage)
    for name in ("macafond", "macafon", "macafon-stt"):
        source = worker if name == "macafon-stt" else ROOT / "target/release" / name
        shutil.copy2(source, stage / f"{name}-{triple}")
        sidecars.append(str(stage / name))
    files = {
        "/usr/lib/systemd/user/macafond.service": str(ROOT / "packaging/systemd/macafond.service"),
        "/usr/share/macafon/cleanup-user-sessions.sh": str(ROOT / "packaging/linux/cleanup-user-sessions.sh"),
        "/usr/share/doc/macafon/LICENSE": str(ROOT / "LICENSE"),
    }
    runtime_files = {f"/usr/lib/macafon/{lib.name}": str(lib) for lib in libraries.iterdir()}
    files.update(runtime_files)
    for icon in (ROOT / "packaging/icons/hicolor").rglob("*"):
        if icon.is_file():
            files[f'/usr/share/icons/hicolor/{icon.relative_to(ROOT / "packaging/icons/hicolor")}'] = str(icon)
    common = {
        "files": files,
        "desktopTemplate": str(ROOT / "packaging/linux/desktop.hbs"),
        "preRemoveScript": str(ROOT / "packaging/linux/prerm.sh"),
        "postInstallScript": str(ROOT / "packaging/linux/postinst.sh"),
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
                "deb": {**common, "section": "sound", "recommends": [
                    "wtype", "xdotool", "ydotool", "wl-clipboard", "xclip",
                    "kscreen", "wlr-randr", "x11-xserver-utils", "xdg-utils"], "depends": [
                    "pipewire (>= 1.0)", "wireplumber (>= 0.5)", "libwebkit2gtk-4.1-0", "libgtk-3-0", "libstdc++6"]},
                "rpm": {**common, "recommends": [
                    "wtype", "xdotool", "ydotool", "wl-clipboard", "xclip",
                    "kscreen", "wlr-randr", "xrandr", "xdg-utils"], "depends": ["pipewire >= 1.0", "wireplumber >= 0.5", "webkit2gtk4.1", "gtk3", "libstdc++"]},
                "appimage": {"bundleMediaFramework": False,
                             "files": {key.lstrip("/"): value for key, value in runtime_files.items()}},
            },
        }
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bundles", default="deb,rpm,appimage")
    parser.add_argument("--bundle-only", action="store_true", help="use existing release binaries")
    parser.add_argument("--build-only", action="store_true", help="build release binaries without packages")
    args = parser.parse_args()
    if sys.platform != "linux" or not shutil.which("pnpm"):
        parser.error("Linux and pnpm are required")
    if args.bundle_only and args.build_only:
        parser.error("choose either --bundle-only or --build-only")
    triple = next(line.split(": ", 1)[1] for line in subprocess.check_output(
        ["rustc", "-vV"], text=True).splitlines() if line.startswith("host:"))
    environment = build_environment()
    stage = ROOT / "target/package-linux"
    stage.mkdir(parents=True, exist_ok=True)
    if not args.bundle_only:
        run("cargo", "build", "--release", "--locked", "-p", "macafond", "-p", "macafon-cli", "-p", "macafon-stt-worker", env=environment)
    notices(stage / "licenses")
    stt_notices(stage / "licenses")
    config = stage / "tauri-bundle.conf.json"
    config.write_text(json.dumps(configuration(stage, triple), indent=2) + "\n")
    ui = ROOT / "apps/macafon-ui"
    if not args.bundle_only:
        run("pnpm", "tauri", "build", "--no-bundle", "--config", config, cwd=ui, env=environment)
    if not args.build_only:
        run("pnpm", "tauri", "bundle", "--bundles", args.bundles, "--config", config, cwd=ui, env=environment)
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
depends=('pipewire>=1.0' 'wireplumber>=0.5' 'webkit2gtk-4.1' 'gtk3' 'libsoup3' 'gcc-libs')
{aur_optdepends()}
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
