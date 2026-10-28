#!/usr/bin/env python3
"""CI 版本号：给日常构建的版本号加上预发布标记、时间戳与提交号，并写入 Cargo.toml。

版本号格式（SemVer 预发布号）：

    <基础版本>-<标记>.<序号>.<UTC时间戳>.<短提交号>

例如：

    0.1.0-beta.128.20261027T191530Z.d996e93
    0.1.0-rc.42.20261027T191530Z.d996e93

用法：

    # 计算版本号并打印（不修改文件）
    python3 scripts/ci-version.py --tag beta --run 128 --sha d996e93

    # 计算版本号并写入 Cargo.toml（同时可写入 GITHUB_OUTPUT）
    python3 scripts/ci-version.py --tag beta --run 128 --sha d996e93 --stamp

    # 直接指定版本号（发布流程用），只写入 Cargo.toml
    python3 scripts/ci-version.py --version 0.1.0 --stamp

说明：
- 基础版本取自 Cargo.toml 的 [workspace.package] version；若其本身带预发布后缀
  （如 0.1.0-rc.1），日常构建只取其数字部分（0.1.0）再拼接 CI 标记。
- 写入 Cargo.toml 后需执行 `cargo update --workspace` 同步 Cargo.lock，
  否则 `cargo build --locked` 会因版本不一致而失败。
"""

from __future__ import annotations

import argparse
import datetime as _dt
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
CARGO_TOML = ROOT / "Cargo.toml"

# Cargo.toml 中 [workspace.package] 段的版本行
_VERSION_LINE = re.compile(r'^(?P<prefix>version\s*=\s*")(?P<value>[^"]*)(?P<suffix>".*)$')


def read_base_version() -> str:
    """读取 [workspace.package] 的 version。"""
    in_section = False
    for line in CARGO_TOML.read_text(encoding="utf-8").splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_section = stripped == "[workspace.package]"
            continue
        if in_section:
            m = _VERSION_LINE.match(line)
            if m:
                return m.group("value")
    sys.exit("错误：未在 Cargo.toml 的 [workspace.package] 中找到 version")


def stamp_cargo_toml(new_version: str) -> None:
    """把 [workspace.package] 的 version 替换为 new_version。"""
    lines = CARGO_TOML.read_text(encoding="utf-8").splitlines(keepends=True)
    in_section = False
    changed = False
    for i, line in enumerate(lines):
        stripped = line.strip()
        if stripped.startswith("["):
            in_section = stripped == "[workspace.package]"
            continue
        if in_section and not changed:
            m = _VERSION_LINE.match(line.rstrip("\r\n"))
            if m:
                newline = "\r\n" if line.endswith("\r\n") else "\n"
                lines[i] = f'{m.group("prefix")}{new_version}{m.group("suffix")}{newline}'
                changed = True
    if not changed:
        sys.exit("错误：未能在 Cargo.toml 的 [workspace.package] 中定位 version 行")
    CARGO_TOML.write_text("".join(lines), encoding="utf-8")


def main() -> None:
    ap = argparse.ArgumentParser(description="计算并写入 CI 版本号")
    ap.add_argument("--tag", default="", help="预发布标记，如 beta / rc；留空表示正式版本")
    ap.add_argument("--run", default="", help="流水号（CI run number 或 PR 号）")
    ap.add_argument("--sha", default="", help="短提交号")
    ap.add_argument("--timestamp", default="", help="UTC 时间戳（默认取当前时间），格式 YYYYMMDDTHHMMSSZ")
    ap.add_argument("--version", default="", help="直接指定版本号（覆盖 --tag/--run/--sha 的拼接）")
    ap.add_argument("--stamp", action="store_true", help="把版本号写入 Cargo.toml")
    ap.add_argument("--output", default="", help="把版本号追加写入该文件（如 $GITHUB_OUTPUT）")
    args = ap.parse_args()

    if args.version:
        version = args.version
    else:
        base = read_base_version().split("-")[0]  # 去掉原有的预发布后缀
        if not args.tag:
            version = base
        else:
            ts = args.timestamp or _dt.datetime.now(_dt.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
            parts = []
            if args.run:
                parts.append(args.run)
            if ts:
                parts.append(ts)
            if args.sha:
                parts.append(args.sha)
            # 基础版本与预发布标记之间用连字符，其后各段用点分隔
            version = f"{base}-{args.tag}" + ("." + ".".join(parts) if parts else "")

    # SemVer 预发布号只允许字母、数字与连字符
    if not re.fullmatch(r"[0-9A-Za-z.\-+]+", version):
        sys.exit(f"错误：版本号含有非法字符：{version}")

    if args.stamp:
        stamp_cargo_toml(version)

    print(version)

    if args.output:
        with open(args.output, "a", encoding="utf-8") as fh:
            fh.write(f"version={version}\n")


if __name__ == "__main__":
    main()
