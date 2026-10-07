#!/usr/bin/env python3
"""Record a real interactive terminal session as an asciicast v2 file.

A real `bash` runs inside a pseudo-terminal. Every command is typed into it one
character at a time, bash echoes it, executes it, and every byte the terminal
receives is stored with its timestamp. Nothing is typeset afterwards: the film
plays these bytes back through a terminal emulator (xterm.js).

    python3 pty_record.py session.json out.cast

session.json: {"cols":96,"rows":26,"cwd":"...","env":{...},"path":[...],
               "prompt":"~/stake-pool", "steps":[{"cmd":"eplyx ...","timeout":300},
               {"pause":0.8}, {"marker":"label"}]}

Each command's exit status is recorded as a marker event ("m", "exit N"),
read from an invisible OSC sequence the prompt emits, which is removed from the
recorded output. Typing delays are real; they are not re-timed here.
"""
import fcntl
import json
import os
import pty
import random
import re
import select
import signal
import struct
import sys
import termios
import time

DONE = re.compile(rb"\x1b\]777;eplyx-done;(\d+)\x07")


def main():
    spec = json.load(open(sys.argv[1]))
    out_path = sys.argv[2]
    cols, rows = spec.get("cols", 96), spec.get("rows", 26)
    env = {
        "PATH": ":".join(spec.get("path", []) + ["/usr/local/bin", "/usr/bin", "/bin"]),
        "HOME": spec.get("home", os.environ.get("HOME", "/root")),
        "TERM": "xterm-256color",
        "LANG": "C.UTF-8",
        "COLUMNS": str(cols),
        "LINES": str(rows),
    }
    # The sandbox reaches the internet only through its egress proxy; pass the
    # proxy and CA settings through (they are not printed by any command).
    for k in ("HTTPS_PROXY", "https_proxy", "NO_PROXY", "no_proxy", "SSL_CERT_FILE",
              "SSL_CERT_DIR", "CURL_CA_BUNDLE", "REQUESTS_CA_BUNDLE", "NODE_EXTRA_CA_CERTS"):
        if os.environ.get(k):
            env[k] = os.environ[k]
    env.update(spec.get("env", {}))
    label = spec.get("prompt", "~")
    # Violet path, lavender arrow; the OSC marker reports $? and is invisible.
    # PROMPT_COMMAND keeps $? of the user's command; a non-zero status is
    # shown in red before the next prompt, exactly as bash reported it.
    env["PROMPT_COMMAND"] = '__e=$?; if [ "$__e" -ne 0 ]; then __x="✗ $__e "; else __x=""; fi'
    env["PS1"] = (
        "\\[\\e]777;eplyx-done;${__e}\\a\\]"
        "\\[\\e[38;2;255;125;140m\\]${__x}\\[\\e[0m\\]"
        f"\\[\\e[38;2;167;139;250m\\]{label}\\[\\e[0m\\] \\[\\e[38;2;231;221;255m\\]❯\\[\\e[0m\\] "
    )
    pid, fd = pty.fork()
    if pid == 0:
        os.chdir(spec["cwd"])
        os.execve("/bin/bash", ["bash", "--noprofile", "--norc", "-i"], env)
    fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", rows, cols, 0, 0))

    start = time.monotonic()
    events = []
    live = open(out_path + ".live", "w")
    pending = b""
    exits = []

    def now():
        return round(time.monotonic() - start, 6)

    def pump(timeout):
        """Read whatever arrives within `timeout`; return list of exit codes seen."""
        nonlocal pending
        seen = []
        deadline = time.monotonic() + timeout
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                break
            r, _, _ = select.select([fd], [], [], left)
            if not r:
                break
            try:
                data = os.read(fd, 65536)
            except OSError:
                break
            if not data:
                break
            pending += data
            # Remove complete markers; keep a possibly partial one for later.
            for m in DONE.finditer(pending):
                seen.append(int(m.group(1)))
            cleaned = DONE.sub(b"", pending)
            cut = cleaned.rfind(b"\x1b]")
            keep = b""
            if cut != -1 and b"\x07" not in cleaned[cut:]:
                keep, cleaned = cleaned[cut:], cleaned[:cut]
            pending = keep
            if cleaned:
                text = cleaned.decode("utf-8", "replace")
                events.append([now(), "o", text])
                live.write(text); live.flush()
            if seen:
                # Drain the rest of the prompt that follows the marker.
                deadline = min(deadline, time.monotonic() + 0.15)
        return seen

    def wait_prompt(timeout):
        end = time.monotonic() + timeout
        while time.monotonic() < end:
            seen = pump(0.25)
            if seen:
                return seen[-1]
        raise SystemExit(f"timeout waiting for prompt in {out_path}")

    wait_prompt(10)  # initial prompt (its $? is meaningless)
    events.clear()
    start = time.monotonic() - 0.4
    os.write(fd, b"clear\r")
    wait_prompt(5)
    events.clear()
    start = time.monotonic() - 0.2
    # Re-draw a fresh prompt at t=0 so the cast starts clean.
    os.write(fd, b"\r")
    wait_prompt(5)
    events[:] = [e for e in events if e[1] != "o" or e[2].strip(" \r\n") not in ("", "\x1b[?2004l", "\x1b[?2004h", "\x1b[?2004l\r")]
    events.insert(0, [0.0, "o", "\x1b[H\x1b[2J"])

    rng = random.Random(7)
    for step in spec["steps"]:
        if "pause" in step:
            pump(step["pause"])
            continue
        if "marker" in step:
            events.append([now(), "m", step["marker"]])
            continue
        cmd = step["cmd"]
        # {{grab:REGEX}} types the last match of REGEX in the output so far,
        # the way an operator copies an id that the previous command printed.
        def grab(m):
            seen = "".join(e[2] for e in events if e[1] == "o")
            found = re.findall(m.group(1), seen)
            if not found:
                raise SystemExit(f"nothing to grab for {m.group(1)}")
            return found[-1]
        cmd = re.sub(r"\{\{grab:([^}]+)\}\}", grab, cmd)
        events.append([now(), "m", "type " + cmd])
        speed = step.get("speed", spec.get("speed", 1.0))
        for ch in cmd:
            os.write(fd, ch.encode())
            pump((0.028 + rng.random() * 0.03 if ch != " " else 0.05) / speed)
        pump(0.35)
        events.append([now(), "m", "run"])
        os.write(fd, b"\r")
        if step.get("hold"):
            # A long-running command (a server): record its output for `hold`
            # seconds, then stop it with Ctrl-C as a user would.
            pump(step["hold"])
            events.append([now(), "m", "interrupt"])
            os.write(fd, b"\x03")
            wait_prompt(10)
            exits.append({"cmd": cmd, "exit": "interrupted"})
            pump(step.get("after", 0.4))
            continue
        code = wait_prompt(step.get("timeout", 600))
        exits.append({"cmd": cmd, "exit": code})
        events.append([now(), "m", f"exit {code}"])
        expected = step.get("expect")
        if expected is not None and code != expected:
            raise SystemExit(f"{cmd!r} exited {code}, expected {expected}")
        pump(step.get("after", 0.6))

    os.kill(pid, signal.SIGHUP)
    header = {"version": 2, "width": cols, "height": rows, "timestamp": int(time.time()),
              "env": {"TERM": "xterm-256color", "SHELL": "/bin/bash"},
              "title": spec.get("title", "")}
    with open(out_path, "w") as f:
        f.write(json.dumps(header) + "\n")
        for e in events:
            f.write(json.dumps(e, ensure_ascii=False) + "\n")
    json.dump(exits, open(out_path + ".exits.json", "w"), indent=1)
    print(f"{out_path}: {len(events)} events, {now():.1f}s, exits {[x['exit'] for x in exits]}")


if __name__ == "__main__":
    main()
