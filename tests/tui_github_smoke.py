"""Opt-in real GitHub add smoke: Linux PTY, public API, temporary packs only.

Run after cargo build --release. Requires network and Python 3.11+.
Downloads are hashed, never executed. Not part of the offline CI suite.
"""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import signal
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time
import tomllib
import urllib.request


def api(path):
    request = urllib.request.Request(
        "https://api.github.com/" + path,
        headers={"User-Agent": "bkmpw-tui-acceptance"},
    )
    with urllib.request.urlopen(request, timeout=20) as response:
        return json.load(response)


def smoke(binary, repository):
    # Freeze expectations from the same first page shown by the picker. A new
    # release during the test fails the identity assertions instead of passing
    # against a different attachment silently.
    release = api(f"repos/{repository}/releases?per_page=20")[0]
    assets = api(f"repos/{repository}/releases/{release['id']}/assets?per_page=20")
    index, asset = next((i, a) for i, a in enumerate(assets) if a["name"].endswith(".jar"))
    digest = asset.get("digest", "")
    assert digest and digest.startswith("sha256:"), "Choose an asset with a published SHA-256"
    expected_hash = digest.removeprefix("sha256:")
    assert asset["size"] < 20_000_000, "Smoke fixture must stay below 20 MB"
    for download in [False, True]:
        with tempfile.TemporaryDirectory(prefix="bkmpw-github-smoke-") as temporary:
            base = Path(temporary).resolve()
            root = base / "中文 pack"
            root.mkdir()
            env = dict(os.environ, XDG_CONFIG_HOME=str(base / "config"),
                       XDG_STATE_HOME=str(base / "state"), XDG_DATA_HOME=str(base / "data"),
                       LANG="en_US.UTF-8", LC_ALL="en_US.UTF-8", TERM="xterm-256color")
            subprocess.run([binary, "init", str(root)], env=env, check=True, capture_output=True)
            original_pack = (root / "pack.toml").read_bytes()
            queue = base / "state/bkmpw/queues" / (hashlib.sha256(str(root).encode()).hexdigest() + ".json")
            master, slave = pty.openpty()
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, 120, 0, 0))
            before = termios.tcgetattr(slave)
            process = subprocess.Popen([binary, "tui", str(root)], env=env,
                                       stdin=slave, stdout=slave, stderr=slave)
            output = bytearray()
            screen_width = 120

            def drain(seconds=0.15):
                deadline = time.monotonic() + seconds
                while time.monotonic() < deadline:
                    if select.select([master], [], [], 0.02)[0]:
                        output.extend(os.read(master, 65536))

            def key(value):
                os.write(master, value)
                drain()

            def wait_for(text, timeout=40):
                # Ask for a full repaint: differential terminal updates may
                # otherwise split a title across several cursor commands.
                nonlocal screen_width
                screen_width = 121 if screen_width == 120 else 120
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 40, screen_width, 0, 0))
                process.send_signal(signal.SIGWINCH)
                def visible():
                    return re.sub(rb"\x1b\[[0-?]*[ -/]*[@-~]", b"", output).replace(b" ", b"")
                expected = text.encode().replace(b" ", b"")
                deadline = time.monotonic() + timeout
                while expected not in visible() and time.monotonic() < deadline:
                    drain()
                assert expected in visible(), f"Did not reach {text!r}: {visible()[-1000:].decode('utf-8', errors='replace')}"


            try:
                drain(0.6)
                key(b"\t")  # Add page
                key(b"g")
                wait_for("GitHub repository")
                key(repository.encode())
                key(b"\r\r")  # Repository field -> Save -> submit
                wait_for("Release (")
                assert release["tag_name"].encode() in output
                key(b"\r\r\r")  # Choice -> page -> Save -> submit
                wait_for("Asset (")
                for _ in range(index):
                    key(b"\x1b[C")
                key(b"\r\r\r")
                wait_for("Add file")
                key(b"\t\t")  # Keep update tag/filter; enter the display name
                key("Smoke 中文".encode())
                key(b"\t\t\t\t")  # filename, kind, side, download
                if download:
                    key(b" ")
                key(b"\t\r")
                wait_for("Review changes")
                assert expected_hash.encode() in output
                assert not list((root / "mods").glob("*.pw.toml"))
                assert (root / "pack.toml").read_bytes() == original_pack
                key(b"\r")  # Explicit preview confirmation starts this fresh queue
                tasks = json.loads(queue.read_text())["tasks"]
                assert len(tasks) == 1, tasks
                assert ("Download" if download else "PreparedEdit") in tasks[0]["request"]
                key(b"\t\t")  # Tasks page remains responsive while work runs
                deadline = time.monotonic() + 60
                while time.monotonic() < deadline:
                    drain()
                    task = json.loads(queue.read_text())["tasks"][0]
                    if task["status"] not in ("Waiting", "Running"):
                        break
                assert task["status"] == "Completed", task
                metadata_files = list((root / "mods").glob("*.pw.toml"))
                assert len(metadata_files) == 1, metadata_files
                metadata = tomllib.loads(metadata_files[0].read_text())
                assert metadata["name"] == "Smoke 中文"
                assert metadata["filename"] == asset["name"]
                assert metadata["update"]["github"]["project"] == repository
                assert metadata["download"]["url"] == asset["browser_download_url"]
                assert metadata["download"]["hash"] == expected_hash
                payload = root / "mods" / asset["name"]
                assert payload.exists() == download
                if download:
                    assert payload.stat().st_size == asset["size"]
                    assert hashlib.sha256(payload.read_bytes()).hexdigest() == expected_hash
                key(b"q")
                process.wait(timeout=5)
                assert process.returncode == 0
                assert termios.tcgetattr(slave) == before
                print(json.dumps(dict(result="PASS", repository=repository,
                                      tag=release["tag_name"], asset_id=asset["id"],
                                      filename=asset["name"], sha256=expected_hash,
                                      download=download), ensure_ascii=False), flush=True)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                os.close(master)
                os.close(slave)


if __name__ == "__main__":
    if not sys.platform.startswith("linux"):
        raise SystemExit("This real-network PTY smoke currently supports Linux only")
    smoke(str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/bkmpw").resolve()),
          sys.argv[2] if len(sys.argv) > 2 else "FabricMC/fabric")
