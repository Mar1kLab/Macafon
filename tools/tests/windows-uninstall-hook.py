#!/usr/bin/env python3
"""Regression: failed audio cleanup must stop NSIS before any files are removed.

Uses only a fake daemon in a temporary directory; never accesses system audio settings.
"""

import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]


@unittest.skipUnless(sys.platform == "win32", "Windows NSIS regression")
class UninstallHook(unittest.TestCase):
    def test_cleanup_failure_aborts_before_deletion(self):
        compiler = Path(os.environ["LOCALAPPDATA"]) / "tauri/NSIS/makensis.exe"
        with tempfile.TemporaryDirectory(prefix="macafon-uninstall-") as directory:
            stage = Path(directory)
            source = stage / "fake.rs"
            source.write_text(
                'fn main() { let exe = std::env::current_exe().expect("exe"); '
                'let ok = exe.parent().expect("parent").join("cleanup-ok").exists(); '
                'std::process::exit(if ok { 0 } else { 1 }); }\n',
                encoding="utf-8",
            )
            subprocess.run(
                ["rustc", str(source), "-o", str(stage / "macafond.exe")], check=True
            )
            script = stage / "harness.nsi"
            hook = ROOT / "packaging/windows/hooks.nsh"
            script.write_text(
                'Name "Macafon cleanup regression"\n'
                'OutFile "harness.exe"\n'
                'RequestExecutionLevel user\n'
                'SilentInstall silent\nSilentUnInstall silent\n'
                f'!include "{hook}"\n'
                'Section\n  WriteUninstaller "$EXEDIR\\uninstall.exe"\nSectionEnd\n'
                'Section "Uninstall"\n'
                '  !insertmacro NSIS_HOOK_PREUNINSTALL\n'
                '  FileOpen $1 "$INSTDIR\\deleted-marker" w\n'
                '  FileClose $1\nSectionEnd\n',
                encoding="utf-8",
            )
            subprocess.run([str(compiler), "/V2", str(script)], check=True, timeout=30)
            subprocess.run([str(stage / "harness.exe"), "/S"], check=True, timeout=15)
            command = [str(stage / "uninstall.exe"), "/S", f"_?={stage}"]
            failed = subprocess.run(command, cwd=stage, timeout=15, check=False)
            self.assertEqual(failed.returncode, 1)
            self.assertFalse((stage / "deleted-marker").exists())
            (stage / "cleanup-ok").touch()
            subprocess.run(command, cwd=stage, timeout=15, check=True)
            self.assertTrue((stage / "deleted-marker").exists())


if __name__ == "__main__":
    unittest.main()
