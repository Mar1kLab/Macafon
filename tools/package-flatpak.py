#!/usr/bin/env python3
"""Stage CI-built Linux binaries; Flatpak assembly performs no network access."""
import json
from pathlib import Path
import shutil
import subprocess

ROOT = Path(__file__).resolve().parents[1]
APP = ROOT / "target/flatpak-payload"


def main():
    if APP.exists():
        shutil.rmtree(APP)
    (APP / "bin").mkdir(parents=True)
    libs = APP / "lib/macafon"
    libs.mkdir(parents=True)
    for name in ("macafon", "macafon-ui", "macafond"):
        shutil.copy2(ROOT / "target/release" / name, APP / "bin" / name)
    runtime = ROOT / "target/package-linux/runtime/usr"
    shutil.copy2(runtime / "bin/macafon-stt", APP / "bin/macafon-stt")
    shutil.copytree(runtime / "lib/macafon", libs, dirs_exist_ok=True)
    # Ship the host PipeWire client and SPA plugins as one compatible set.
    multiarch = subprocess.check_output(["gcc", "-print-multiarch"], text=True).strip()
    for directory in ("pipewire-0.3", "spa-0.2"):
        shutil.copytree(Path('/usr/lib') / multiarch / directory, APP / "lib" / directory)
    # Only supplement libraries missing from the GNOME runtime. Never replace its
    # glibc, GTK or WebKit with a second distribution's copy.
    location = subprocess.check_output(
        ["flatpak", "info", "--user", "--show-location", "org.gnome.Platform//49"], text=True).strip()
    available = {p.name for p in (Path(location) / "files/lib").rglob("*.so*")}
    binaries = [p for p in APP.rglob('*') if p.is_file() and p.read_bytes()[:4] == b'\x7fELF']
    for binary in binaries:
        output = subprocess.check_output(["ldd", str(binary)], text=True)
        for line in output.splitlines():
            parts = line.split()
            if len(parts) >= 3 and parts[1] == '=>' and parts[2].startswith('/'):
                soname, source = parts[0], Path(parts[2])
                if soname not in available or soname.startswith(('libpipewire-', 'libspa-')):
                    shutil.copy2(source, libs / soname)
    for binary in [p for p in APP.rglob('*') if p.is_file() and p.read_bytes()[:4] == b'\x7fELF']:
        subprocess.run(['patchelf', '--set-rpath', '/app/lib/macafon', str(binary)], check=True)
    share = APP / 'share'
    (share / 'applications').mkdir(parents=True)
    (share / 'applications/dev.macafon.ui.desktop').write_text(
        '[Desktop Entry]\nType=Application\nName=Macafon\nExec=macafon\nIcon=dev.macafon.ui\nCategories=AudioVideo;Audio;Mixer;\nTerminal=false\n')
    for icon in (ROOT / 'packaging/icons/hicolor').glob('*/apps/macafon.png'):
        dest = share / 'icons/hicolor' / icon.parent.parent.name / 'apps/dev.macafon.ui.png'
        dest.parent.mkdir(parents=True, exist_ok=True)
        shutil.copy2(icon, dest)
    shutil.copytree(ROOT / 'target/package-linux/licenses', share / 'macafon/licenses')
    shutil.copy2(ROOT / 'packaging/flatpak/check-runtime.sh', share / 'macafon/check-runtime.sh')
    shutil.copy2(ROOT / 'LICENSE', share / 'macafon/LICENSE')


if __name__ == '__main__':
    main()
