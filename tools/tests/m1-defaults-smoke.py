#!/usr/bin/env python3
"""M1 default microphone ownership, Quit, crash recovery and offline uninstall."""
import subprocess
from m1_private import Client, PrivateGraph, ROOT, smoke


def main():
    with PrivateGraph() as graph:
        daemon = graph.daemon()
        client = Client(graph.socket_path)
        journal = graph.directory / "state/macafon/default-source.json"
        initial = graph.defaults()
        client.request("mic.set_default_source", {"confirmed": True})
        client.request("mic.restore_default_source")
        assert graph.defaults() == initial, "absent default keys were not restored"
        graph.default("original_mic")
        client.request("mic.set_default_source", {"confirmed": True})
        for key in ("default.configured.audio.source", "default.audio.source"):
            graph.run("pw-metadata", "-d", "-n", "default", "0", key)
        client.request("mic.restore_default_source")
        assert "default.audio.source" not in graph.defaults(), "user reset was overwritten"
        graph.default("original_mic")
        try:
            graph.run(ROOT / "target/release/macafon", "ctl", "set-default")
            raise AssertionError("CLI changed defaults without confirmation")
        except subprocess.CalledProcessError as error:
            assert "--confirm" in error.stderr
        client.request("mic.set_default_source", {"confirmed": False}, error=True)
        assert graph.defaults()["default.audio.source"]["name"] == "original_mic"
        assert not journal.exists()
        client.request("mic.set_default_source", {"confirmed": True})
        assert journal.is_file() and journal.stat().st_mode & 0o777 == 0o600
        assert graph.defaults()["default.audio.source"]["name"] == "macafon_mic"
        client.request("mic.set_default_source", {"confirmed": True})
        client.request("daemon.quit")
        client.close()
        daemon.wait(timeout=8)
        assert graph.defaults()["default.audio.source"]["name"] == "original_mic"
        assert not journal.exists()

        daemon = graph.daemon()
        client = Client(graph.socket_path)
        client.request("mic.set_default_source", {"confirmed": True})
        graph.default("user_later_mic")
        client.request("daemon.quit")
        client.close()
        daemon.wait(timeout=8)
        assert graph.defaults()["default.audio.source"]["name"] == "user_later_mic"

        daemon = graph.daemon()
        client = Client(graph.socket_path)
        client.request("mic.set_default_source", {"confirmed": True})
        graph.stop(daemon, kill=True)
        client.close()
        assert journal.exists()
        daemon = graph.daemon()
        smoke.wait_for(lambda: not journal.exists(), "crash recovery restored journal")
        assert graph.defaults()["default.audio.source"]["name"] == "user_later_mic"
        client = Client(graph.socket_path)
        client.request("mic.set_default_source", {"confirmed": True})
        graph.stop(daemon, kill=True)
        client.close()
        graph.run(ROOT / "target/release/macafon", "restore-default")
        assert graph.defaults()["default.audio.source"]["name"] == "user_later_mic"
        assert not journal.exists()
        daemon = graph.daemon()
        client = Client(graph.socket_path)
        client.request("mic.set_default_source", {"confirmed": True})
        graph.stop(graph.server)
        client.request("daemon.quit")
        client.close()
        daemon.wait(timeout=8)
        assert journal.exists(), "offline shutdown forgot recovery journal"
        graph.server = graph.spawn(["pipewire", "-c", graph.directory / "pipewire.conf"])
        smoke.wait_for(lambda: (graph.runtime / "pipewire-0").exists(), "restarted private PipeWire")
        graph.run(ROOT / "target/release/macafon", "restore-default")
        assert graph.defaults()["default.audio.source"]["name"] == "user_later_mic"
        assert not journal.exists()
        print("defaults: confirmation, durable ownership, absent keys, repeated set, Quit, later user choice/reset, crash recovery, offline shutdown and standalone uninstall restore passed")


if __name__ == "__main__":
    main()
