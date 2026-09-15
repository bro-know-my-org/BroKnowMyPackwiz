"""Offline release TUI/queue smoke test using an isolated Linux PTY session."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import select
import struct
import subprocess
import sys
import tempfile
import termios
import time


def smoke(binary):
    with tempfile.TemporaryDirectory(prefix="bkmpw-tui-smoke-") as temporary:
        base = Path(temporary).resolve()
        root = base / "中文 pack"
        root.mkdir()
        env = os.environ.copy()
        env.update(
            XDG_CONFIG_HOME=str(base / "config"),
            XDG_STATE_HOME=str(base / "state"),
            XDG_DATA_HOME=str(base / "data"),
            LANG="en_US.UTF-8",
            TERM="xterm-256color",
        )
        subprocess.run([binary, "init", str(root)], env=env, check=True, capture_output=True)
        (root / "roots/client").rmdir()
        (root / ".gitignore").write_text("*.jar\n.bkmpw/\n", encoding="utf-8")
        (root / "mods/manual.jar").write_bytes(b"manual")
        payload = b"managed payload"
        (root / "mods/managed.jar").write_bytes(payload)
        digest = hashlib.sha256(payload).hexdigest()
        (root / "mods/managed.pw.toml").write_text(
            'name = "Managed"\nfilename = "managed.jar"\nside = "both"\n'
            '[download]\nurl = "https://example.invalid/managed.jar"\n'
            f'hash-format = "sha256"\nhash = "{digest}"\n', encoding="utf-8"
        )
        (root / "templates").mkdir()
        (root / "templates/README.md").write_text("Smoke pack\n", encoding="utf-8")
        with (root / ".pw/config.toml").open("a", encoding="utf-8") as config:
            config.write('\n[release]\ntemplate-dir = "templates"\ntemplate-files = "README.md"\n')
        subprocess.run([binary, "refresh", str(root)], env=env, check=True, capture_output=True)
        # Create a real saved queue through the TUI before testing restart.
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
        before = termios.tcgetattr(slave)
        process = subprocess.Popen([binary, "tui", str(root)], stdin=slave, stdout=slave, stderr=slave, env=env)

        def drain(seconds=0.15):
            deadline = time.monotonic() + seconds
            while time.monotonic() < deadline:
                if select.select([master], [], [], 0.02)[0]:
                    os.read(master, 65536)

        def key(value):
            os.write(master, value)
            drain()

        try:
            drain(0.6)
            key(b"q")
            process.wait(timeout=5)
            assert process.returncode == 0
            assert termios.tcgetattr(slave) == before
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)
            os.close(slave)
        state = base / "state/bkmpw"
        queue_path = state / "queues" / (hashlib.sha256(str(root).encode()).hexdigest() + ".json")
        assert queue_path.exists(), "This queue smoke requires isolated XDG state (Linux)"
        kinds = ["Init", "Inspect", "List", "Scan", "Check", "Refresh", "DownloadFiles", "Sync",
                 "InstallLocal", "PreparePack", "PrepareServer", "Modlist", "ExportClient",
                 "ExportServer", "ExportServerInstaller", "ExportCurseForge", "Hash"]
        tasks = []
        for index, kind in enumerate(kinds):
            request = dict(kind=kind, output=str(base / "installed") if kind == "InstallLocal" else None,
                           input=str(root / "pack.toml") if kind == "Hash" else None,
                           side="both", root_dir="", algorithm="sha256", names=[], jobs=None,
                           retries=None, retry_delay_seconds=None, force=False, cleanup=None)
            tasks.append(dict(id=f"12345-{index}", label=kind, request={"Command": request},
                              status="Waiting", error=None))
        queue_path.write_text(json.dumps(dict(version=1, root=str(root), tasks=tasks)), encoding="utf-8")
        master, slave = pty.openpty()
        fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, 120, 0, 0))
        before = termios.tcgetattr(slave)
        process = subprocess.Popen([binary, "tui", str(root)], stdin=slave, stdout=slave, stderr=slave, env=env)
        try:
            drain(0.6)
            assert all(task["status"] == "Waiting" for task in json.loads(queue_path.read_text())["tasks"])
            for _ in range(3):
                key(b"\t")
            key(b" ")
            deadline = time.monotonic() + 45
            while time.monotonic() < deadline:
                drain()
                tasks = json.loads(queue_path.read_text())["tasks"]
                failed = [task for task in tasks if task["status"] in ("Failed", "Conflict", "Cancelled")]
                assert not failed, [(task["label"], task["error"], task.get("logs")) for task in failed]
                if all(task["status"] == "Completed" for task in tasks):
                    break
            assert all(task["status"] == "Completed" for task in tasks), tasks
            assert (root / "roots/client").is_dir()
            assert (root / "mods/manual.jar").read_bytes() == b"manual"
            assert (base / "installed/mods/managed.jar").read_bytes() == payload
            assert (root / "README.md").read_text() == "Smoke pack\n"
            for name in ("client-full.zip", "server-pack.zip", "server-installer.zip", "curseforge-export.zip"):
                assert (root / name).stat().st_size > 0, name
            assert all(task.get("logs") for task in tasks)
            key(b"q")
            process.wait(timeout=5)
            assert process.returncode == 0
            assert termios.tcgetattr(slave) == before
            print(f"PASS: {len(tasks)} offline command tasks, restart confirmation, Unicode paths, manual files, terminal restoration")
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
            os.close(master)
            os.close(slave)


if __name__ == "__main__":
    if not sys.platform.startswith("linux"):
        raise SystemExit("Queue smoke currently supports Linux XDG isolation only")
    smoke(str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/bkmpw").resolve()))
