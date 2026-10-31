#!/usr/bin/env python3
"""Linux packaging regressions; never operate on the installed user service."""
import importlib.util
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[2]
SPEC = importlib.util.spec_from_file_location("package_linux", ROOT / "tools/package-linux.py")
PACKAGE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(PACKAGE)


class LinuxPackaging(unittest.TestCase):
    def test_builds_use_bundled_cpu_bindings_without_changing_parent_environment(self):
        with tempfile.TemporaryDirectory() as directory, \
                patch.dict(os.environ, {"WHISPER_DONT_GENERATE_BINDINGS": "parent"}), \
                patch.object(PACKAGE, "ROOT", Path(directory)), \
                patch.object(PACKAGE.shutil, "which", return_value="pnpm"), \
                patch.object(PACKAGE.subprocess, "check_output", return_value="host: x86_64-unknown-linux-gnu\n"), \
                patch.object(PACKAGE, "notices"), patch.object(PACKAGE, "stt_notices"), \
                patch.object(PACKAGE, "configuration", return_value={}), \
                patch.object(PACKAGE, "run") as run, \
                patch("sys.argv", ["package-linux.py", "--build-only"]):
            PACKAGE.main()
            self.assertEqual(len(run.call_args_list), 2)
            self.assertEqual(run.call_args_list[0].args[:2], ("cargo", "build"))
            self.assertEqual(run.call_args_list[1].args[:3], ("pnpm", "tauri", "build"))
            for call in run.call_args_list:
                self.assertEqual(call.kwargs["env"]["WHISPER_DONT_GENERATE_BINDINGS"], "1")
            self.assertEqual(os.environ["WHISPER_DONT_GENERATE_BINDINGS"], "parent")

    def test_both_native_packages_include_the_postinstall_hook(self):
        with tempfile.TemporaryDirectory() as directory, \
                patch.object(PACKAGE.shutil, "copy2"):
            stage = Path(directory)
            libraries = stage / "libraries"
            libraries.mkdir()
            with patch.object(PACKAGE, "stage_worker", return_value=(stage / "worker", libraries)):
                config = PACKAGE.configuration(stage, "x86_64-unknown-linux-gnu")
            for package in ["deb", "rpm"]:
                hook = config["bundle"]["linux"][package]["postInstallScript"]
                self.assertEqual(Path(hook), ROOT / "packaging/linux/postinst.sh")

    def test_postinstall_refreshes_active_services_without_enabling_autostart(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            bus = socket.socket(socket.AF_UNIX)
            self.addCleanup(bus.close)
            bus.bind(str(root / "bus"))
            log = root / "calls"
            for name, body in {
                "loginctl": "echo '0 root'; echo '1000 testuser'; echo 'invalid ignored'",
                "runuser": 'echo "$*" >> "$TEST_LOG"',
            }.items():
                path = root / name
                path.write_text("#!/bin/sh\n" + body + "\n")
                path.chmod(0o755)
            # Substitute only the runtime directory; all session tools are fake.
            script = (ROOT / "packaging/linux/postinst.sh").read_text().replace(
                '/run/user/$uid', directory)
            environment = {**os.environ, "PATH": str(root) + os.pathsep + os.defpath,
                           "TEST_LOG": str(log)}
            for argument in ["configure", "1", "2"]:
                log.unlink(missing_ok=True)
                subprocess.run(["sh", "-s", "--", argument], input=script,
                               text=True, env=environment, check=True)
                calls = log.read_text().splitlines()
                self.assertEqual(len(calls), 2)
                self.assertTrue(calls[0].endswith("systemctl --user daemon-reload"))
                self.assertTrue(calls[1].endswith("systemctl --user try-restart macafond.service"))
                self.assertTrue(all(line.startswith("-u testuser ") for line in calls))
            log.unlink()
            subprocess.run(["sh", "-s", "--", "abort-upgrade"], input=script,
                           text=True, env=environment, check=True)
            self.assertFalse(log.exists())


if __name__ == "__main__":
    unittest.main()
