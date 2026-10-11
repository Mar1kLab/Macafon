"""Private PipeWire fixture for M1 control tests; never connects to the user graph."""
import importlib.util
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import tempfile

ROOT = Path(__file__).resolve().parents[2]
spec = importlib.util.spec_from_file_location("pw_smoke", ROOT / "tools/tests/pipewire-smoke.py")
smoke = importlib.util.module_from_spec(spec)
spec.loader.exec_module(smoke)


class Client:
    def __init__(self, path, name="macafon-cli"):
        self.socket = socket.socket(socket.AF_UNIX)
        self.socket.settimeout(7)
        self.socket.connect(str(path))
        self.reader = self.socket.makefile("r")
        self.sequence = 0
        self.request("hello", {"protocol": 1, "client": name})

    def request(self, method, params=None, error=False):
        self.sequence += 1
        message = {"id": self.sequence, "method": method}
        if params is not None:
            message["params"] = params
        self.socket.sendall((json.dumps(message) + "\n").encode())
        while True:
            line = self.reader.readline()
            assert line, "daemon closed the connection"
            reply = json.loads(line)
            if reply.get("id") == self.sequence:
                if error:
                    assert "error" in reply, reply
                    return reply["error"]
                assert "error" not in reply, reply
                return reply["result"]

    def close(self):
        self.reader.close()
        self.socket.close()


class PrivateGraph:
    def __enter__(self):
        self.temp = tempfile.TemporaryDirectory(prefix="macafon-m1-private-")
        self.directory = Path(self.temp.name)
        self.runtime = self.directory / "runtime"
        self.runtime.mkdir(mode=0o700)
        self.env = dict(os.environ, XDG_RUNTIME_DIR=str(self.runtime),
                        PIPEWIRE_RUNTIME_DIR=str(self.runtime), PIPEWIRE_REMOTE="pipewire-0",
                        XDG_CONFIG_HOME=str(self.directory / "config"),
                        XDG_STATE_HOME=str(self.directory / "state"))
        self.processes = []
        self.logs = []
        self.socket_path = self.runtime / "macafon/macafond.sock"
        config = self.directory / "pipewire.conf"
        config.write_text(smoke.CONFIG.replace('node.description = "Test sink"',
                                               'node.description = "Test sink"\n        device.form-factor = speaker'))
        self.server = self.spawn(["pipewire", "-c", config])
        smoke.wait_for(lambda: (self.runtime / "pipewire-0").exists(), "private PipeWire")
        return self

    def spawn(self, command):
        log = (self.directory / f"process-{len(self.logs)}.log").open("w+")
        self.logs.append(log)
        process = subprocess.Popen([str(arg) for arg in command], env=self.env,
                                   stdout=log, stderr=log)
        self.processes.append(process)
        return process

    def run(self, *command):
        return subprocess.check_output([str(arg) for arg in command], env=self.env,
                                       text=True, stderr=subprocess.PIPE, timeout=8)

    def daemon(self):
        process = self.spawn([ROOT / "target/release/macafon", "daemon", "--no-tray",
                              "--input", "test_microphone", "--preset", "raw"])
        smoke.wait_for(lambda: self.socket_path.exists(), "daemon socket")
        smoke.wait_for(lambda: any(item.get("info", {}).get("props", {}).get("node.name") == "macafon_mic"
                                  for item in json.loads(self.run("pw-dump"))), "virtual mic")
        return process

    def defaults(self):
        data = json.loads(self.run("pw-dump"))
        # pw-dump omits metadata objects without properties even though registry binding works.
        metadata = next((item for item in data if item["type"] == "PipeWire:Interface:Metadata"
                         and item.get("props", {}).get("metadata.name") == "default"), {})
        return {item["key"]: item.get("value") for item in metadata.get("metadata", [])
                if item["subject"] == 0}

    def default(self, name):
        for key in ("default.configured.audio.source", "default.audio.source"):
            self.run("pw-metadata", "-n", "default", "0", key, json.dumps({"name": name}), "Spa:String:JSON")

    @staticmethod
    def stop(process, kill=False):
        if process.poll() is None:
            process.send_signal(signal.SIGKILL if kill else signal.SIGTERM)
            process.wait(timeout=7)

    def __exit__(self, kind, value, traceback):
        if kind:
            for log in self.logs:
                log.flush()
                log.seek(0)
                print(log.read())
        for process in reversed(self.processes):
            self.stop(process)
        for log in self.logs:
            log.close()
        self.temp.cleanup()
