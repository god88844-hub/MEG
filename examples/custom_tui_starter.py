"""Bring-Your-Own-TUI Starter Client for MEG (`meg ui-bridge`).

Connects any custom Python/Textual/Rich/Web frontend to MEG's full Aegis Ring-0
harness over NDJSON stdin/stdout without touching Rust.

Usage:
    python examples/custom_tui_starter.py
"""

from __future__ import annotations

import json
import os
import subprocess
from pathlib import Path


def find_meg() -> Path:
    repo = Path(__file__).resolve().parent.parent
    exe = "meg.exe" if os.name == "nt" else "meg"
    for p in (repo / "meg-rs" / "target" / "release" / exe, repo / "meg-rs" / "target" / "debug" / exe):
        if p.is_file():
            return p
    raise FileNotFoundError("Build meg first: cd meg-rs && cargo build --release -p meg-cli")


def main() -> None:
    proc = subprocess.Popen(
        [str(find_meg()), "ui-bridge"],
        stdin=subprocess.PIPE,
        stdout=subprocess.PIPE,
        text=True,
        bufsize=1,
    )
    assert proc.stdin and proc.stdout
    ready = json.loads(proc.stdout.readline())
    print(f"Connected to MEG UI Bridge: {ready['protocol']} | workspace={ready['workspace']}")

    # Example: assign per-role models (Antigravity-style) and query status
    proc.stdin.write(json.dumps({
        "cmd": "set_roles",
        "planner": "gemini-3.1-pro",
        "builder": "qwen2.5-coder:14b",
        "reviewer": "claude-opus-4-6",
    }) + "\n")
    proc.stdin.flush()
    print("Roles:", proc.stdout.readline().strip())

    proc.stdin.write(json.dumps({"cmd": "quit"}) + "\n")
    proc.stdin.flush()
    print("Closed:", proc.stdout.readline().strip())
    proc.wait(timeout=5)


if __name__ == "__main__":
    main()