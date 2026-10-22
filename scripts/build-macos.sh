#!/usr/bin/env bash
# macOS 本机构建：仅 Apple Silicon（M 系列，aarch64-apple-darwin），打成 dist/macos-arm64/LaborFee.app 与 zip。
#
# 用法：scripts/build-macos.sh [--desktop-only] [--no-sign]
#   --desktop-only  只构建桌面程序（默认同时构建命令行 labor-fee 与 labor-fee-web）
#   --no-sign       不做 ad-hoc 签名（默认 codesign -s -；Apple Silicon 要求可执行文件至少有 ad-hoc 签名才能运行）
#
# 产物不支持 Intel Mac。在 Intel Mac 上也可以运行本脚本交叉编译出 arm64 版本（脚本会自动 rustup target add），
# 但产物只能拷到 M 系列 Mac 上运行。
# 依赖：Xcode Command Line Tools（xcode-select --install，提供 clang、codesign）、Rust 1.95+（rustup）。
# 在 Linux 上用 osxcross 交叉构建见 scripts/cross-build-docker.sh macos。
# 未经 Apple 公证（notarize），首次打开需在“系统设置 → 隐私与安全性”中允许，或右键“打开”。
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/common.sh"

TARGET="aarch64-apple-darwin"
ARCH_LABEL="arm64"
DESKTOP_ONLY=0 SIGN=1
while [[ $# -gt 0 ]]; do
    case "$1" in
        --desktop-only) DESKTOP_ONLY=1; shift ;;
        --no-sign) SIGN=0; shift ;;
        --arch)
            case "${2:-}" in
                aarch64|arm64) shift 2 ;;
                *) die "macOS 只构建 Apple Silicon（arm64），不再支持 --arch ${2:-}（universal/x86_64 已取消）" ;;
            esac ;;
        -h|--help) sed -n '2,12p' "$0"; exit 0 ;;
        *) die "未知参数 $1" ;;
    esac
done

need cargo "请先安装 Rust：https://rustup.rs"
if [[ "$(uname -s)" == "Darwin" && "$(uname -m)" == "x86_64" ]]; then
    # 注意：Rosetta 下的终端也会报告 x86_64
    if [[ "$(sysctl -n sysctl.proc_translated 2>/dev/null || echo 0)" == "1" ]]; then
        log "提示：当前终端运行在 Rosetta 下，仍将构建原生 arm64 版本。"
    else
        log "提示：这是 Intel Mac。将交叉编译 Apple Silicon（arm64）版本，产物不能在本机运行，请拷到 M 系列 Mac 上使用。"
    fi
fi
export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"

BINS=(labor-fee-desktop)
PKGS=(-p labor-fee-desktop)
if [[ $DESKTOP_ONLY -eq 0 ]]; then BINS+=(labor-fee labor-fee-web); PKGS+=(-p labor-fee-cli -p labor-fee-web); fi

cd "$ROOT" || exit 1
TDIR="${CARGO_TARGET_DIR:-$ROOT/target}"
ensure_target "$TARGET"
log "构建 $TARGET"
cargo build --release --locked "${PKGS[@]}" --target "$TARGET"

OUT="$DIST/macos-$ARCH_LABEL"
rm -rf "$OUT" && mkdir -p "$OUT"
for b in "${BINS[@]}"; do
    cp "$TDIR/$TARGET/release/$b" "$OUT/$b"
    chmod +x "$OUT/$b"
done
if command -v file >/dev/null 2>&1; then file "$OUT/labor-fee-desktop"; fi

APP="$(make_app "$OUT/labor-fee-desktop" "$OUT")"
if [[ $SIGN -eq 1 ]] && command -v codesign >/dev/null 2>&1; then
    log "ad-hoc 签名"
    codesign --force --deep --sign - "$APP"
    for b in "${BINS[@]}"; do [[ "$b" == labor-fee-desktop ]] || codesign --force --sign - "$OUT/$b"; done
elif [[ $SIGN -eq 1 ]] && command -v rcodesign >/dev/null 2>&1; then
    # 非 macOS（如 osxcross 交叉构建）时用 apple-codesign 的 rcodesign 做 ad-hoc 签名（不带证书即为 ad-hoc）
    log "ad-hoc 签名（rcodesign）"
    rcodesign sign "$APP"
    for b in "${BINS[@]}"; do [[ "$b" == labor-fee-desktop ]] || rcodesign sign "$OUT/$b"; done
elif [[ $SIGN -eq 1 ]]; then
    log "提示：未找到 codesign / rcodesign，未做 ad-hoc 签名。若在 M 系列 Mac 上无法启动，请执行：codesign --force --deep -s - LaborFee.app"
fi
copy_docs "$OUT"
ZIP="$DIST/labor-fee-$VERSION-macos-$ARCH_LABEL.zip"
rm -f "$ZIP"
if command -v ditto >/dev/null 2>&1; then
    ditto -c -k --sequesterRsrc --keepParent "$OUT" "$ZIP"
else
    need zip "打包需要 ditto（macOS 自带）或 zip。"
    (cd "$DIST" && zip -qry "$ZIP" "$(basename "$OUT")")
fi
log "完成：$APP"
log "压缩包：$ZIP"
