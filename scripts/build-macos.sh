#!/usr/bin/env bash
# macOS 本机构建：默认 universal（x86_64 + aarch64，lipo 合并），并打成 dist/macos/LaborFee.app 与 zip。
#
# 用法：scripts/build-macos.sh [--arch universal|x86_64|aarch64] [--desktop-only] [--no-sign]
#   --arch          默认 universal
#   --desktop-only  只构建桌面程序（默认同时构建命令行 labor-fee 与 labor-fee-web）
#   --no-sign       不做 ad-hoc 签名（默认 codesign -s -，便于在 Apple Silicon 上直接运行）
#
# 依赖：Xcode Command Line Tools（xcode-select --install，提供 clang、lipo、codesign）、Rust 1.95+（rustup）。
# 在 Linux 上用 osxcross 交叉构建见 scripts/cross-build-docker.sh macos。
# 未经 Apple 公证（notarize），首次打开需在“系统设置 → 隐私与安全性”中允许，或右键“打开”。
source "$(dirname "$0")/lib/common.sh"

ARCH_MODE="universal" DESKTOP_ONLY=0 SIGN=1
while [[ $# -gt 0 ]]; do
    case "$1" in
        --arch) ARCH_MODE="$2"; shift 2 ;;
        --desktop-only) DESKTOP_ONLY=1; shift ;;
        --no-sign) SIGN=0; shift ;;
        -h|--help) sed -n '2,13p' "$0"; exit 0 ;;
        *) die "未知参数 $1" ;;
    esac
done

need cargo "请先安装 Rust：https://rustup.rs"
LIPO="${LIPO:-lipo}"
case "$ARCH_MODE" in
    universal) TARGETS=(x86_64-apple-darwin aarch64-apple-darwin) ;;
    x86_64) TARGETS=(x86_64-apple-darwin) ;;
    aarch64|arm64) TARGETS=(aarch64-apple-darwin); ARCH_MODE=aarch64 ;;
    *) die "--arch 只能是 universal / x86_64 / aarch64" ;;
esac
[[ ${#TARGETS[@]} -gt 1 ]] && need "$LIPO" "universal 构建需要 lipo（Xcode Command Line Tools），或设置 LIPO 环境变量。"
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"

BINS=(labor-fee-desktop)
PKGS=(-p labor-fee-desktop)
if [[ $DESKTOP_ONLY -eq 0 ]]; then BINS+=(labor-fee labor-fee-web); PKGS+=(-p labor-fee-cli -p labor-fee-web); fi

cd "$ROOT"
TDIR="${CARGO_TARGET_DIR:-$ROOT/target}"
for t in "${TARGETS[@]}"; do
    ensure_target "$t"
    log "构建 $t"
    cargo build --release --locked "${PKGS[@]}" --target "$t"
done

OUT="$DIST/macos-$ARCH_MODE"
rm -rf "$OUT" && mkdir -p "$OUT"
for b in "${BINS[@]}"; do
    if [[ ${#TARGETS[@]} -gt 1 ]]; then
        log "lipo 合并 $b"
        "$LIPO" -create -output "$OUT/$b" "$TDIR/${TARGETS[0]}/release/$b" "$TDIR/${TARGETS[1]}/release/$b"
    else
        cp "$TDIR/${TARGETS[0]}/release/$b" "$OUT/$b"
    fi
    chmod +x "$OUT/$b"
done
command -v "$LIPO" >/dev/null 2>&1 && "$LIPO" -info "$OUT/labor-fee-desktop" || true

APP="$(make_app "$OUT/labor-fee-desktop" "$OUT")"
if [[ $SIGN -eq 1 ]] && command -v codesign >/dev/null 2>&1; then
    log "ad-hoc 签名"
    codesign --force --deep --sign - "$APP"
fi
copy_docs "$OUT"
ZIP="$DIST/labor-fee-$VERSION-macos-$ARCH_MODE.zip"
rm -f "$ZIP"
if command -v ditto >/dev/null 2>&1; then
    ditto -c -k --sequesterRsrc --keepParent "$OUT" "$ZIP"
else
    (cd "$DIST" && zip -qry "$ZIP" "$(basename "$OUT")")
fi
log "完成：$APP"
log "压缩包：$ZIP"
