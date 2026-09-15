"""Opt-in authenticated CurseForge smoke using Linux release PTY sessions.

Requires CURSEFORGE_API_KEY, Python 3.11+, and network. Never prints the key.
Uses AppleSkin + required Fabric API on Minecraft 1.21.1 in temporary packs.
"""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import select
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time
import tomllib
import urllib.parse
import urllib.request


def smoke(binary):
    secret = os.environ.get("CURSEFORGE_API_KEY", "").strip()
    assert secret, "Set CURSEFORGE_API_KEY before running the real-network smoke"

    def api(path):
        request = urllib.request.Request("https://api.curseforge.com/v1/" + path,
                                        headers={"x-api-key": secret, "User-Agent": "bkmpw-tui-acceptance"})
        with urllib.request.urlopen(request, timeout=20) as response:
            return json.load(response)["data"]

    query = urllib.parse.urlencode(dict(gameId=432, classId=6, searchFilter="AppleSkin",
                                       index=0, pageSize=30, sortField=2, sortOrder="desc",
                                       gameVersion="1.21.1", modLoaderType=4))
    projects = api("mods/search?" + query)
    project_index = next(i for i, project in enumerate(projects) if project["id"] == 248787)
    expected = {}

    def resolve(project_id):
        if project_id in expected:
            return
        files = api(f"mods/{project_id}/files?index=0&pageSize=30&gameVersion=1.21.1&modLoaderType=4")
        candidate = next(f for f in files if "1.21.1" in f["gameVersions"] and "Fabric" in f["gameVersions"])
        expected[project_id] = candidate
        assert candidate["fileLength"] < 20_000_000, "Keep the smoke fixture below 20 MB per file"
        for dependency in candidate.get("dependencies", []):
            if dependency["relationType"] == 3:
                resolve(dependency["modId"])

    resolve(248787)
    assert 306612 in expected, "Fixture must exercise a real required dependency"
    root_file = expected[248787]
    for mode in ["cancel", "metadata", "download"]:
        with tempfile.TemporaryDirectory(prefix="bkmpw-curseforge-smoke-") as temporary:
            base = Path(temporary).resolve()
            root = base / "中文 pack"
            root.mkdir()
            env = dict(os.environ, CURSEFORGE_API_KEY=secret, XDG_CONFIG_HOME=str(base / "config"),
                       XDG_STATE_HOME=str(base / "state"), XDG_DATA_HOME=str(base / "data"),
                       LANG="en_US.UTF-8", LC_ALL="en_US.UTF-8", TERM="xterm-256color")
            subprocess.run([binary, "init", str(root)], env=env, check=True, capture_output=True)
            (root / "pack.toml").write_text('name = "CF smoke"\n[versions]\nminecraft = "1.21.1"\nfabric = "0.16.14"\n')
            original_pack = (root / "pack.toml").read_bytes()
            queue = base / "state/bkmpw/queues" / (hashlib.sha256(str(root).encode()).hexdigest() + ".json")
            master, slave = pty.openpty()
            width = 120
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, width, 0, 0))
            before = termios.tcgetattr(slave)
            process = subprocess.Popen([binary, "tui", str(root)], env=env,
                                       stdin=slave, stdout=slave, stderr=slave)
            output = bytearray()

            def drain(seconds=0.15):
                deadline = time.monotonic() + seconds
                while time.monotonic() < deadline:
                    if select.select([master], [], [], 0.02)[0]:
                        output.extend(os.read(master, 65536))

            def key(value):
                os.write(master, value)
                drain()

            def visible():
                return re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", output).replace(b" ", b"")

            def wait_for(text):
                nonlocal width
                width = 121 if width == 120 else 120
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, width, 0, 0))
                process.send_signal(signal.SIGWINCH)
                expected_text = text.encode().replace(b" ", b"")
                deadline = time.monotonic() + 60
                while expected_text not in visible() and time.monotonic() < deadline:
                    drain()
                assert expected_text in visible(), f"{mode}: did not reach {text!r}"

            try:
                drain(0.6)
                key(b"\t/")
                key(b"AppleSkin\r")
                # The query text alone is not evidence that results arrived.
                wait_for("curseforge.com")
                for _ in range(project_index):
                    key(b"\x1b[B")
                key(b"\r")
                wait_for(root_file["fileName"])
                key(b"\r")
                wait_for("Preview required dependencies")
                key(b"\t")
                if mode == "download":
                    key(b" ")
                key(b"\t\r")
                wait_for("Confirm metadata and dependencies")
                for file in expected.values():
                    wait_for(file["fileName"])
                assert not list((root / "mods").glob("*.pw.toml"))
                assert (root / "pack.toml").read_bytes() == original_pack
                if mode == "cancel":
                    key(b"\x1b")
                    key(b"q")
                    process.wait(timeout=5)
                    assert json.loads(queue.read_text())["tasks"] == []
                    assert not list((root / "mods").glob("*.pw.toml"))
                    assert (root / "pack.toml").read_bytes() == original_pack
                else:
                    key(b"\r")  # One explicit confirmation for root and all dependencies
                    key(b"\t\t")  # Tasks remains usable during downloads
                    deadline = time.monotonic() + 90
                    task = None
                    while time.monotonic() < deadline:
                        drain()
                        tasks = json.loads(queue.read_text())["tasks"]
                        if tasks and tasks[0]["status"] not in ("Waiting", "Running"):
                            assert len(tasks) == 1
                            task = tasks[0]
                            break
                    assert task and task["status"] == "Completed", "CurseForge task did not complete"
                    assert ("Download" if mode == "download" else "PreparedEdit") in task["request"]
                    metadata = [tomllib.loads(path.read_text()) for path in (root / "mods").glob("*.pw.toml")]
                    assert {m["update"]["curseforge"]["project-id"] for m in metadata} == set(expected)
                    for entry in metadata:
                        cf = entry["update"]["curseforge"]
                        file = expected[cf["project-id"]]
                        digest = next(h["value"] for h in file["hashes"] if h["algo"] == 1)
                        assert cf["file-id"] == file["id"]
                        assert entry["filename"] == file["fileName"]
                        assert entry["download"]["mode"] == "metadata:curseforge"
                        assert entry["download"]["hash-format"] == "sha1"
                        assert entry["download"]["hash"] == digest
                        payload = root / "mods" / file["fileName"]
                        assert payload.exists() == (mode == "download")
                        if mode == "download":
                            assert payload.stat().st_size == file["fileLength"]
                            assert hashlib.sha1(payload.read_bytes()).hexdigest() == digest
                    key(b"q")
                    process.wait(timeout=5)
                assert process.returncode == 0
                assert termios.tcgetattr(slave) == before
                assert secret.encode() not in queue.read_bytes(), "API key leaked into queue history"
                assert secret.encode() not in output, "API key leaked into terminal output"
                print(json.dumps(dict(result="PASS", mode=mode, files=[
                    dict(project_id=project_id, file_id=file["id"], filename=file["fileName"],
                         sha1=next(h["value"] for h in file["hashes"] if h["algo"] == 1))
                    for project_id, file in expected.items()]), ensure_ascii=False), flush=True)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                os.close(master)
                os.close(slave)


if __name__ == "__main__":
    if not sys.platform.startswith("linux"):
        raise SystemExit("Authenticated CurseForge PTY smoke currently supports Linux only")
    smoke(str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/bkmpw").resolve()))
