#!/usr/bin/env python3
"""Compile runtime Luau sources and enforce their strict-mode headers."""

import argparse
from pathlib import Path
import shutil
import subprocess
import sys


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", default="luau-compile")
    args = parser.parse_args()
    compiler = shutil.which(args.compiler)
    if compiler is None:
        print(f"Luau compiler not found: {args.compiler}", file=sys.stderr)
        return 1
    compiler = str(Path(compiler).resolve())

    root = Path(__file__).resolve().parent.parent
    files = sorted(
        path
        for pattern in ("*.luau", "blocks/**/*.luau", "lib/**/*.luau")
        for path in root.glob(pattern)
        if path.is_file() and not path.name.endswith(".d.luau")
    )
    if not files:
        print("No runtime Luau sources found.", file=sys.stderr)
        return 1

    failed = False
    for path in files:
        with path.open(encoding="utf-8") as source:
            header = source.readline().rstrip("\r\n")
        if header != "--!strict":
            print(
                f"{path.relative_to(root)}:1: expected --!strict on the first line",
                file=sys.stderr,
            )
            failed = True
            continue
        result = subprocess.run(
            [compiler, "--null", str(path)],
            cwd=root,
            stdin=subprocess.DEVNULL,
            check=False,
        )
        if result.returncode != 0:
            failed = True

    if failed:
        return 1
    print(f"Checked {len(files)} runtime Luau files.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
