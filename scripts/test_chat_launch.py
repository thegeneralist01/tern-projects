#!/usr/bin/env python3
"""Run host-launch regressions with the Luau CLI (same release as luau-compile)."""

import argparse
import json
from pathlib import Path
import shutil
import subprocess
import tempfile


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--runtime", default="luau")
    args = parser.parse_args()
    runtime = shutil.which(args.runtime)
    if runtime is None:
        parser.error(f"Luau runtime not found: {args.runtime}")
    root = Path(__file__).resolve().parent.parent
    modules = ("lib/resume", "lib/chat-launch", "blocks/panel", "window")
    sources = "local sources = {\n" + "".join(
        f"[{json.dumps(name)}] = {json.dumps((root / (name + '.luau')).read_text(encoding='utf-8'), ensure_ascii=False)},\n"
        for name in modules
    ) + "}\n"
    harness = (root / "scripts/chat-launch-regression.luau").read_text(encoding="utf-8")
    with tempfile.TemporaryDirectory(prefix="tern-chat-launch-") as temp:
        script = Path(temp) / "regression.luau"
        script.write_text(sources + harness, encoding="utf-8")
        return subprocess.run([runtime, str(script)], check=False).returncode


if __name__ == "__main__":
    raise SystemExit(main())
