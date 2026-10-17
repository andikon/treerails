"""Controlled Windows/Unix PTY proof; never invokes Copilot or authenticates.
Build first: cargo build --example copilot_probe
Run: python scripts/copilot_probe_smoke.py
"""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parent.parent
EXE = ROOT / "target/debug/examples" / ("copilot_probe.exe" if os.name == "nt" else "copilot_probe")


def request(endpoint, **command):
    ep = json.loads(endpoint.read_text(encoding="utf-8"))
    host, port = ep["address"].split(":")
    with socket.create_connection((host, int(port)), timeout=5) as stream:
        stream.sendall((json.dumps(dict(token=ep["token"], **command)) + "\n").encode())
        if command["command"] == "stop":
            return None
        return json.loads(stream.makefile(encoding="utf-8").readline())


def main():
    with tempfile.TemporaryDirectory(prefix="treerail-probe-") as folder:
        endpoint = Path(folder) / "owner.json"
        # Regular files avoid inherited capture pipes in detached Windows trees.
        with (Path(folder) / "launcher.log").open("wb") as output:
            subprocess.run([str(EXE), "fixture-start", str(endpoint), str(ROOT)], stdout=output, stderr=output, check=True, timeout=15)
        try:
            deadline = time.monotonic() + 10
            while True:
                first = request(endpoint, command="snapshot")
                if "CONTROLLED CHILD WAITING" in first["screen"]:
                    break
                if time.monotonic() > deadline:
                    raise AssertionError("Detached output did not drain")
                time.sleep(.05)
            assert first["bytes"] > 100_000
            assert first["alternate_screen"]
            assert first["attention"]["value"] is None, "Fixture must not impersonate provider attention"
            request(endpoint, command="resize", rows=22, cols=70)
            request(endpoint, command="input", text="probe-input\r\n")
            time.sleep(.2)
            second = request(endpoint, command="snapshot")
            assert first["pid"] == second["pid"]
            assert len(second["screen"].splitlines()) <= 22
            assert "probe-input" in second["screen"] or "112, 114, 111, 98, 101" in second["screen"]
            view = subprocess.run([str(EXE), "view-smoke", str(endpoint)], capture_output=True, text=True, check=True, timeout=10)
            view = [json.loads(line) for line in view.stdout.splitlines()]
            assert all(item["same_pid"] and item["view_bytes"] > 0 for item in view)
            print(json.dumps({"controlled_child": "passed", "same_pid": first["pid"], "drained_bytes": first["bytes"], "alternate_screen": True, "resize_and_input": True, "embedded_view_detach": view}))
        finally:
            request(endpoint, command="stop")
            deadline = time.monotonic() + 5
            while endpoint.exists() and time.monotonic() < deadline:
                time.sleep(.05)
            assert not endpoint.exists(), "Owner did not stop"


if __name__ == "__main__":
    main()
