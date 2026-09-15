"""Offline Linux release PTY smoke for external-editor handoff and recovery."""
import fcntl
import hashlib
import json
import os
from pathlib import Path
import pty
import re
import select
import shlex
import signal
import struct
import subprocess
import sys
import tempfile
import termios
import time


EDITOR = '''import json, pathlib, sys, termios
probe, mode, source = sys.argv[1:]
source = pathlib.Path(source)
flags = termios.tcgetattr(0)[3]
text = source.read_text(encoding="utf-8")
source.write_text("name = [" if mode == "invalid" else text.replace('"Original"', '"Edited 中文"'), encoding="utf-8")
pathlib.Path(probe).write_text(json.dumps(dict(source=str(source), flags=flags)), encoding="utf-8")
print("EDITOR_RETURNED", flush=True)
sys.exit(7 if mode == "fail" else 0)
'''


def smoke(binary):
    for mode in ["apply", "cancel", "fail", "invalid", "conflict"]:
        with tempfile.TemporaryDirectory(prefix="bkmpw-editor-smoke-") as temporary:
            base = Path(temporary).resolve()
            root = base / "中文 pack"
            root.mkdir()
            env = dict(os.environ, XDG_CONFIG_HOME=str(base / "config"),
                       XDG_STATE_HOME=str(base / "state"), XDG_DATA_HOME=str(base / "data"),
                       LANG="en_US.UTF-8", LC_ALL="en_US.UTF-8", TERM="xterm-256color")
            subprocess.run([binary, "init", str(root)], env=env, check=True, capture_output=True)
            metadata = root / "mods/original.pw.toml"
            original = ('# keep this comment\nname = "Original"\nfilename = "example.jar"\n'
                        'side = "both"\ncustom = "keep"\n[download]\n'
                        'url = "https://example.invalid/example.jar"\nhash-format = "sha256"\n'
                        f'hash = "{"0" * 64}"\n')
            metadata.write_text(original, encoding="utf-8")
            editor = base / "测试 editor.py"
            editor.write_text(EDITOR, encoding="utf-8")
            probe = base / "probe.json"
            preferences = base / "config/bkmpw/tui.json"
            preferences.parent.mkdir(parents=True)
            preferences.write_text(json.dumps({"language": "En", "editor": shlex.join(
                [sys.executable, str(editor), str(probe), mode])}), encoding="utf-8")
            queue = base / "state/bkmpw/queues" / (hashlib.sha256(str(root).encode()).hexdigest() + ".json")
            master, slave = pty.openpty()
            width = 120
            fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, width, 0, 0))
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
                fcntl.ioctl(slave, termios.TIOCSWINSZ, struct.pack("HHHH", 32, width, 0, 0))
                process.send_signal(signal.SIGWINCH)
                expected = text.encode().replace(b" ", b"")
                deadline = time.monotonic() + 10
                while expected not in visible() and time.monotonic() < deadline:
                    drain()
                assert expected in visible(), f"{mode}: did not reach {text!r}"

            try:
                drain(0.6)
                wait_for("Original")
                key(b"E")
                wait_for({"fail": "Editor failed", "invalid": "Editor returned invalid TOML"}
                         .get(mode, "Review changes"))
                record = json.loads(probe.read_text())
                draft = Path(record["source"])
                assert draft.is_relative_to(base / "state/bkmpw/drafts")
                assert draft != metadata and draft.is_file()
                assert record["flags"] & (termios.ICANON | termios.ECHO) == before[3] & (termios.ICANON | termios.ECHO)
                assert not termios.tcgetattr(slave)[3] & termios.ICANON, "TUI must reactivate raw input"
                assert metadata.read_text(encoding="utf-8") == original
                if mode in ("fail", "invalid", "cancel"):
                    key(b"\x1b")
                    # A failed editor or cancelled review must never enqueue a write.
                    key(b"q")
                    process.wait(timeout=5)
                    assert json.loads(queue.read_text())["tasks"] == []
                    assert metadata.read_text(encoding="utf-8") == original
                    if mode in ("fail", "invalid"):
                        assert draft.is_file(), "Failed editor drafts remain available for repair"
                else:
                    external = original.replace('"Original"', '"External"')
                    if mode == "conflict":
                        metadata.write_text(external, encoding="utf-8")
                    key(b"\r")  # Confirm the staged edit
                    deadline = time.monotonic() + 10
                    task = None
                    while time.monotonic() < deadline:
                        drain()
                        tasks = json.loads(queue.read_text())["tasks"]
                        if tasks and tasks[0]["status"] not in ("Waiting", "Running"):
                            assert len(tasks) == 1
                            task = tasks[0]
                            break
                    assert task is not None, "Edit task did not finish"
                    if mode == "conflict":
                        assert task["status"] in ("Failed", "Conflict"), task
                        assert metadata.read_text(encoding="utf-8") == external
                    else:
                        assert task["status"] == "Completed", task
                        edited = metadata.read_text(encoding="utf-8")
                        assert 'name = "Edited 中文"' in edited
                        assert '# keep this comment' in edited and 'custom = "keep"' in edited
                    key(b"q")
                    process.wait(timeout=5)
                assert process.returncode == 0
                assert termios.tcgetattr(slave) == before
                print(f"PASS: external editor {mode}, private draft, cooked/raw handoff, terminal restoration", flush=True)
            finally:
                if process.poll() is None:
                    process.kill()
                    process.wait()
                os.close(master)
                os.close(slave)


if __name__ == "__main__":
    if not sys.platform.startswith("linux"):
        raise SystemExit("External-editor PTY smoke currently supports Linux only")
    smoke(str(Path(sys.argv[1] if len(sys.argv) > 1 else "target/release/bkmpw").resolve()))
