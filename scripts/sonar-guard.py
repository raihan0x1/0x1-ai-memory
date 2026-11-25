#!/usr/bin/env python3
"""Static SonarCloud guardrails for Rust, GitHub Actions, and install.sh."""

from __future__ import annotations

import pathlib
import re
import sys

RUST_MODE_RE = re.compile(
    r"\b(?:from_mode|set_mode)\s*\(\s*(0o[0-7_]+|0x[0-9a-fA-F_]+|[0-9]+)\s*\)"
)
ASYNC_FN_RE = re.compile(r"\basync\s+fn\b")
SPAWN_BLOCKING_RE = re.compile(r"\bspawn_blocking\s*\(")
BLOCKING_IN_ASYNC_RE = re.compile(
    r"\bstd::process::Command\b"
    r"|\bstd::thread::sleep\b"
    r"|\bstd::fs::(?!Permissions\b|Metadata\b|FileType\b|OpenOptions\b)[a-z_][a-z0-9_]*\b"
)
WORKFLOW_USES_RE = re.compile(r"^\s*(?:-\s*)?uses:\s*([^\s#]+)")
SHA_PIN_RE = re.compile(r"@[0-9a-f]{40}\b")


def line_number(src: str, pos: int) -> int:
    return src.count("\n", 0, pos) + 1


def skip_line_comment(src: str, idx: int, out: list[str]) -> int:
    n = len(src)
    while idx < n and src[idx] != "\n":
        out.append(" ")
        idx += 1
    return idx


def skip_block_comment(src: str, idx: int, out: list[str]) -> int:
    n = len(src)
    depth = 1
    out.append("  ")
    idx += 2
    while idx < n and depth > 0:
        pair = src[idx : idx + 2]
        if pair == "/*":
            depth += 1
            out.append("  ")
            idx += 2
        elif pair == "*/":
            depth -= 1
            out.append("  ")
            idx += 2
        else:
            out.append("\n" if src[idx] == "\n" else " ")
            idx += 1
    return idx


def match_raw_string_prefix(src: str, idx: int) -> tuple[int, str] | None:
    if idx > 0 and (src[idx - 1].isalnum() or src[idx - 1] == "_"):
        return None
    n = len(src)
    cursor = idx
    if src[cursor] in ("b", "c") and cursor + 1 < n and src[cursor + 1] == "r":
        cursor += 1
    elif src[cursor] != "r":
        return None

    cursor += 1
    hashes = 0
    while cursor < n and src[cursor] == "#":
        hashes += 1
        cursor += 1
    if cursor < n and src[cursor] == '"':
        return cursor + 1, '"' + ("#" * hashes)
    return None


def skip_raw_string(
    src: str, idx: int, content_start: int, end_seq: str, out: list[str]
) -> int:
    out.append(" " * (content_start - idx))
    cursor = content_start
    n = len(src)
    while cursor < n:
        if src.startswith(end_seq, cursor):
            out.append(" " * len(end_seq))
            return cursor + len(end_seq)
        out.append("\n" if src[cursor] == "\n" else " ")
        cursor += 1
    return cursor


def skip_quoted_string(src: str, idx: int, out: list[str]) -> int:
    n = len(src)
    out.append(" ")
    idx += 1
    while idx < n:
        ch = src[idx]
        if ch == "\\" and idx + 1 < n:
            out.append("\n" if src[idx + 1] == "\n" else "  ")
            idx += 2
        elif ch == '"':
            out.append(" ")
            return idx + 1
        else:
            out.append("\n" if ch == "\n" else " ")
            idx += 1
    return idx


def skip_char_literal(src: str, idx: int, out: list[str]) -> int | None:
    n = len(src)
    if idx + 2 >= n:
        return None
    if src[idx + 1] == "\\":
        cursor = idx + 2
        while cursor < n and src[cursor] not in ("'", "\n"):
            cursor += 1
        if cursor < n and src[cursor] == "'":
            out.append(" " * (cursor - idx + 1))
            return cursor + 1
        return None
    if src[idx + 2] == "'" and src[idx + 1] != "\n":
        out.append("   ")
        return idx + 3
    return None


def strip_rust_comments_and_strings(src: str) -> str:
    """Strip Rust comments and literals while keeping line numbers identical."""
    out: list[str] = []
    idx = 0
    n = len(src)
    while idx < n:
        ch = src[idx]
        nxt = src[idx + 1] if idx + 1 < n else ""
        if ch == "/" and nxt == "/":
            idx = skip_line_comment(src, idx, out)
            continue
        if ch == "/" and nxt == "*":
            idx = skip_block_comment(src, idx, out)
            continue
        raw = match_raw_string_prefix(src, idx) if ch in ("r", "b", "c") else None
        if raw is not None:
            idx = skip_raw_string(src, idx, raw[0], raw[1], out)
            continue
        if ch == '"':
            idx = skip_quoted_string(src, idx, out)
            continue
        if ch == "'":
            end_char = skip_char_literal(src, idx, out)
            if end_char is not None:
                idx = end_char
                continue
        out.append(ch)
        idx += 1
    return "".join(out)


def find_matching_delim(src: str, open_idx: int, open_ch: str, close_ch: str) -> int:
    depth = 0
    for idx in range(open_idx, len(src)):
        ch = src[idx]
        if ch == open_ch:
            depth += 1
        elif ch == close_ch:
            depth -= 1
            if depth == 0:
                return idx
    return len(src) - 1


def mask_spawn_blocking(body: str) -> str:
    masked = list(body)
    for match in SPAWN_BLOCKING_RE.finditer(body):
        open_paren = body.find("(", match.start())
        if open_paren == -1:
            continue
        end_paren = find_matching_delim(body, open_paren, "(", ")")
        for idx in range(open_paren, end_paren + 1):
            if masked[idx] != "\n":
                masked[idx] = " "
    return "".join(masked)


def check_rust_permissions(path: pathlib.Path, clean: str) -> list[str]:
    """Enforce rust:S2612: disallow world permissions or group-write in from_mode/set_mode."""
    errors: list[str] = []
    for match in RUST_MODE_RE.finditer(clean):
        val = int(match.group(1).replace("_", ""), 0)
        if (val & 0o007) != 0 or (val & 0o020) != 0:
            ln = line_number(clean, match.start())
            errors.append(
                f"{path}:{ln}: [rust:S2612] loose POSIX mode {match.group(1)} ({oct(val)}); use owner-only 0o700/0o600"
            )
    return errors


def check_rust_async_blocking(path: pathlib.Path, clean: str) -> list[str]:
    """Enforce rust:S7487: disallow blocking std::fs/std::process/std::thread::sleep in async fn."""
    errors: list[str] = []
    for match in ASYNC_FN_RE.finditer(clean):
        brace = clean.find("{", match.end())
        semi = clean.find(";", match.end())
        if brace == -1 or (semi != -1 and semi < brace):
            continue
        end = find_matching_delim(clean, brace, "{", "}")
        body_masked = mask_spawn_blocking(clean[brace : end + 1])
        for hit in BLOCKING_IN_ASYNC_RE.finditer(body_masked):
            ln = line_number(clean, brace + hit.start())
            errors.append(
                f"{path}:{ln}: [rust:S7487] blocking call `{hit.group(0)}` inside async fn; use tokio async API or spawn_blocking"
            )
    return errors


def check_workflows() -> list[str]:
    """Check third-party action SHA pinning and cargo --locked in GitHub workflows."""
    errors: list[str] = []
    wf_dir = pathlib.Path(".github/workflows")
    if not wf_dir.is_dir():
        return errors
    for path in sorted(wf_dir.glob("*.yml")):
        for idx, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
            stripped = line.lstrip()
            if stripped.startswith("#"):
                continue
            uses_m = WORKFLOW_USES_RE.match(line)
            if uses_m:
                target = uses_m.group(1)
                if not target.startswith("actions/") and not SHA_PIN_RE.search(target):
                    errors.append(
                        f"{path}:{idx}: third-party action `{target}` must be pinned to a 40-char commit SHA"
                    )
            if "cargo build" in line and "--release" in line and "--locked" not in line:
                errors.append(
                    f"{path}:{idx}: `cargo build --release` must pass `--locked`"
                )
    return errors


def check_installer() -> list[str]:
    """Enforce HTTPS/TLS 1.2 flags on curl invocations in install.sh."""
    errors: list[str] = []
    installer = pathlib.Path("install.sh")
    if not installer.is_file():
        return errors
    for idx, line in enumerate(installer.read_text(encoding="utf-8").splitlines(), start=1):
        stripped = line.lstrip()
        if stripped.startswith("#"):
            continue
        if re.search(r"\bcurl\s", stripped) and (
            "--proto '=https'" not in stripped or "--tlsv1.2" not in stripped
        ):
            errors.append(
                f"{installer}:{idx}: curl invocation must include `--proto '=https' --tlsv1.2`"
            )
    return errors


def main() -> int:
    errors: list[str] = []
    rust_files = sorted(
        list(pathlib.Path("src").rglob("*.rs")) + list(pathlib.Path("tests").rglob("*.rs"))
    )
    for path in rust_files:
        clean = strip_rust_comments_and_strings(path.read_text(encoding="utf-8"))
        errors.extend(check_rust_permissions(path, clean))
        errors.extend(check_rust_async_blocking(path, clean))
    errors.extend(check_workflows())
    errors.extend(check_installer())

    if errors:
        print("Sonar static guardrail violations:", file=sys.stderr)
        for err in errors:
            print(f"  - {err}", file=sys.stderr)
        return 1

    print("Sonar static guardrails passed.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
