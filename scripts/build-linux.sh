#!/usr/bin/env bash
# Linux 本机构建：生成 dist/linux-<arch>/ 下的 labor-fee-desktop、labor-fee、labor-fee-web 及 tar.gz。
#
# 用法：scripts/build-linux.sh [--target <rust-target>] [--features portal] [--desktop-only]
#   --target        默认本机（x86_64-unknown-linux-gnu / aarch64-unknown-linux-gnu）
#   --features      portal：文件对话框改用 xdg-desktop-portal（不依赖 GTK）；默认 gtk3
#   --desktop-only  只构建桌面程序
#
# 依赖：Rust 1.95+（rustup）、gcc、pkg-config；默认 gtk3 后端需要 libgtk-3-dev
#   Debian/Ubuntu：sudo apt install build-essential pkg-config libgtk-3-dev
#   Fedora：       sudo dnf install gcc pkgconf-pkg-config gtk3-devel
# 运行时依赖：libGL（Mesa）、libxkbcommon、X11 或 Wayland；gtk3 后端另需 libgtk-3-0。
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/common.sh"

TARGET="" FEATURES="" DESKTOP_ONLY=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --target) TARGET="$2"; shift 2 ;;
        --features) FEATURES="$2"; shift 2 ;;
        --desktop-only) DESKTOP_ONLY=1; shift ;;
        -h|--help) sed -n '2,13p' "$0"; exit 0 ;;
        *) die "未知参数 $1" ;;
    esac
done

need cargo "请先安装 Rust：https://rustup.rs"
[[ -z "$TARGET" ]] && TARGET="$(uname -m)-unknown-linux-gnu"
ARCH="${TARGET%%-*}"
ensure_target "$TARGET"

FEATURE_ARGS=()
if [[ "$FEATURES" == "portal" ]]; then
    FEATURE_ARGS=(--no-default-features --features portal)
else
    need pkg-config "请安装 pkg-config。"
    pkg-config --exists gtk+-3.0 || die "缺少 GTK3 开发包（libgtk-3-dev / gtk3-devel），或改用 --features portal。"
fi

cd "$ROOT" || exit 1
TDIR="${CARGO_TARGET_DIR:-$ROOT/target}"
log "构建桌面程序（$TARGET）"
cargo build --release --locked -p labor-fee-desktop --target "$TARGET" "${FEATURE_ARGS[@]}"
BINS=(labor-fee-desktop)
if [[ $DESKTOP_ONLY -eq 0 ]]; then
    log "构建命令行与 Web 服务"
    cargo build --release --locked -p labor-fee-cli -p labor-fee-web --target "$TARGET"
    BINS+=(labor-fee labor-fee-web)
fi

OUT="$DIST/linux-$ARCH"
rm -rf "$OUT" && mkdir -p "$OUT"
for b in "${BINS[@]}"; do
    install -m 755 "$TDIR/$TARGET/release/$b" "$OUT/$b"
done
copy_docs "$OUT"
# 图标与桌面入口（可复制到 ~/.local/share/applications/ 与 ~/.local/share/icons/hicolor/256x256/apps/）
install -m 644 "$ROOT/crates/desktop/assets/icon/icon-256.png" "$OUT/labor-fee-desktop.png"
cat >"$OUT/labor-fee-desktop.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=$APP_TITLE
Comment=劳务报酬增值税、附加税、个税预扣计算
Exec=labor-fee-desktop
Icon=labor-fee-desktop
Categories=Office;Finance;
StartupWMClass=labor-fee-desktop
Terminal=false
DESKTOP
write_checksums "$OUT"
TAR="$DIST/labor-fee-$VERSION-linux-$ARCH.tar.gz"
tar -C "$DIST" -czf "$TAR" "linux-$ARCH"
log "完成：$OUT"
ls -l "$OUT"
log "压缩包：$TAR"
