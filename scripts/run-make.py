#!/usr/bin/env python3
"""run-make: timeout-aware helper around the repository root Makefile."""

from __future__ import annotations

import argparse
import subprocess
import sys
import os
import signal
from pathlib import Path
from typing import List, Sequence

DEFAULT_TIMEOUT = 60
ROOT = Path(__file__).resolve().parents[1]
MAKEFILE = ROOT / "Makefile"


def _collect_phony_targets(makefile_path: Path) -> List[str]:
    """Parse the root Makefile and return all declared phony targets."""
    if not makefile_path.exists():
        raise FileNotFoundError(f"Makefile not found at {makefile_path}")

    targets: List[str] = []
    collecting = False
    parts: List[str] = []

    with makefile_path.open(encoding="utf-8") as handle:
        for raw_line in handle:
            stripped = raw_line.strip()

            if stripped.startswith(".PHONY:"):
                collecting = True
                parts = [stripped[len(".PHONY:") :].strip()]
                if not stripped.endswith("\\"):
                    collecting = False
                    parts = [p for p in parts if p]
                    targets.extend(_extract_tokens(parts))
                    parts = []
                continue

            if collecting and (raw_line.startswith(" ") or raw_line.startswith("\t")):
                parts.append(stripped)
                if not stripped.endswith("\\"):
                    collecting = False
                    targets.extend(_extract_tokens(parts))
                    parts = []
                continue

            if collecting:
                collecting = False
                if parts:
                    targets.extend(_extract_tokens(parts))
                    parts = []

    # Flush if the file ended while still collecting
    if collecting and parts:
        targets.extend(_extract_tokens(parts))

    return sorted(set(targets))


def _extract_tokens(parts: Sequence[str]) -> List[str]:
    """Split concatenated .PHONY lines into individual tokens."""
    joined = " ".join(parts).replace("\\", " ")
    return [token for token in joined.split() if token]


def _build_parser(available_targets: Sequence[str]) -> argparse.ArgumentParser:
    epilog_lines = ["Available commands:"] + [
        f"  - {target}" for target in available_targets
    ]

    parser = argparse.ArgumentParser(
        prog="run-make",
        description=(
            "Run a root Makefile command with an optional timeout.\n"
            "Additional arguments after the command are forwarded to make."
        ),
        epilog="\n".join(epilog_lines),
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=DEFAULT_TIMEOUT,
        help=f"Abort the command after N seconds (default: {DEFAULT_TIMEOUT}).",
    )
    parser.add_argument(
        "command",
        help="Name of the Makefile command to run.",
    )
    parser.add_argument(
        "command_args",
        nargs=argparse.REMAINDER,
        help="Extra parameters forwarded to make (e.g. PORT=/dev/ttyUSB0).",
    )
    return parser


def _run_make(target: str, args: Sequence[str], timeout: int) -> int:
    cmd = ["make", target, *args]
    try:
        process = subprocess.Popen(
            cmd,
            cwd=str(ROOT),
            start_new_session=True,
        )
    except OSError as exc:
        print(
            f"error: failed to launch make for '{target}': {exc}",
            file=sys.stderr,
        )
        return 1

    timed_out = False
    try:
        return_code = process.wait(timeout=timeout)
    except subprocess.TimeoutExpired:
        timed_out = True
        return_code = None
        for sig, wait_after in (
            (signal.SIGINT, 3),  # mimic Ctrl+C first
            (signal.SIGTERM, 2),
            (signal.SIGKILL, None),
        ):
            try:
                os.killpg(process.pid, sig)
            except ProcessLookupError:
                break

            if wait_after is None:
                break

            try:
                return_code = process.wait(timeout=wait_after)
                break
            except subprocess.TimeoutExpired:
                continue

        if return_code is None:
            return_code = process.wait()

    if timed_out:
        print(
            f"timeout: command '{target}' terminated after {timeout} seconds.",
            file=sys.stdout,
        )
        return 0

    if return_code != 0:
        print(
            f"error: make command '{target}' exited with status {return_code}.",
            file=sys.stderr,
        )
    return return_code


def main(argv: Sequence[str] | None = None) -> int:
    targets = _collect_phony_targets(MAKEFILE)

    parser = _build_parser(targets)

    if not targets:
        parser.error("no phony targets detected in Makefile.")

    parsed = parser.parse_args(argv)

    if parsed.timeout <= 0:
        parser.error("--timeout must be a positive integer.")

    target = parsed.command
    if target not in targets:
        target_list = ", ".join(targets)
        print(
            f"error: unknown command '{target}'.\n"
            f"Available commands: {target_list}",
            file=sys.stderr,
        )
        return 2

    # argparse passes the remainder including a leading '--' separator; drop it.
    forwarded_args = list(parsed.command_args)
    if forwarded_args and forwarded_args[0] == "--":
        forwarded_args = forwarded_args[1:]

    return _run_make(target, forwarded_args, parsed.timeout)


if __name__ == "__main__":
    sys.exit(main())
