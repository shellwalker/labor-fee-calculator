#!/usr/bin/env bash
# 构建脚本公共函数（由 build-*.sh / cross-build-docker.sh 引用）。
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck disable=SC2034  # 由引用本文件的构建脚本使用
DIST="$ROOT/dist"
APP_NAME="labor-fee-desktop"
APP_TITLE="劳务报酬计算工具"
BUNDLE_ID="cloud.medway.labor-fee-desktop"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' "$ROOT/Cargo.toml" | head -1)"

log() { printf '\033[1;34m==>\033[0m %s\n' "$*"; }
die() { printf '\033[1;31m错误：\033[0m%s\n' "$*" >&2; exit 1; }
need() { command -v "$1" >/dev/null 2>&1 || die "缺少命令 $1。$2"; }

# 确保 rustup 已安装目标平台
ensure_target() {
    local t="$1"
    if command -v rustup >/dev/null 2>&1; then
        rustup target list --installed | grep -qx "$t" || { log "安装 Rust 目标 $t"; rustup target add "$t"; }
    fi
}

# 写一份随包说明与字体许可证
copy_docs() {
    local dest="$1"
    cp "$ROOT/crates/desktop/assets/fonts/OFL.txt" "$dest/字体许可证-OFL.txt"
    # shellcheck disable=SC1111  # 说明文字中的中文引号是有意的
    cat >"$dest/说明.txt" <<TXT
$APP_TITLE $VERSION

labor-fee-desktop  桌面程序（正算 / 反算 / 按总成本 / 批量 / 口径）
labor-fee          命令行工具（如随包提供）
labor-fee-web      Web 服务（如随包提供）

自定义口径：labor-fee-desktop --config 口径.toml，或在程序“口径”页签加载。
界面内嵌字体为 Noto Sans CJK SC 子集（SIL OFL 1.1），见 字体许可证-OFL.txt。
TXT
}

# 生成 macOS .app 包：make_app <可执行文件> <输出目录>
make_app() {
    local exe="$1" outdir="$2"
    local app="$outdir/LaborFee.app"
    rm -rf "$app"
    mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
    cp "$exe" "$app/Contents/MacOS/$APP_NAME"
    chmod +x "$app/Contents/MacOS/$APP_NAME"
    cp "$ROOT/crates/desktop/assets/fonts/OFL.txt" "$app/Contents/Resources/"
    cat >"$app/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key><string>zh_CN</string>
    <key>CFBundleExecutable</key><string>$APP_NAME</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
    <key>CFBundleName</key><string>LaborFee</string>
    <key>CFBundleDisplayName</key><string>$APP_TITLE</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>11.0</string>
    <key>NSHighResolutionCapable</key><true/>
    <key>LSApplicationCategoryType</key><string>public.app-category.finance</string>
</dict>
</plist>
PLIST
    printf 'APPL????' >"$app/Contents/PkgInfo"
    echo "$app"
}

# 生成 sha256 校验文件
write_checksums() {
    local dir="$1" f files=()
    for f in "$dir"/*; do
        [[ -f "$f" && "$(basename "$f")" != SHA256SUMS ]] && files+=("$(basename "$f")")
    done
    [[ ${#files[@]} -gt 0 ]] || return 0
    if command -v sha256sum >/dev/null 2>&1; then
        (cd "$dir" && sha256sum "${files[@]}" >SHA256SUMS.tmp && mv SHA256SUMS.tmp SHA256SUMS)
    else
        (cd "$dir" && shasum -a 256 "${files[@]}" >SHA256SUMS.tmp && mv SHA256SUMS.tmp SHA256SUMS)
    fi
}
