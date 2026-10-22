#!/usr/bin/env bash
# 可选：在容器中交叉构建（Docker 或 Podman），产物输出到 dist/。
#
# 用法：scripts/cross-build-docker.sh <linux|windows|macos|all> [选项]
#   linux    x86_64-unknown-linux-gnu（容器内 Debian bookworm + GTK3）→ dist/linux-x86_64/
#   windows  x86_64-pc-windows-gnu（mingw-w64）→ dist/windows-gnu-x86_64/*.exe
#   macos    仅 aarch64-apple-darwin（Apple Silicon / M 系列，osxcross，打成 .app）→ dist/macos-arm64/
#            需要自备 macOS SDK 打包文件（例如 MacOSX14.5.sdk.tar.xz，按 osxcross 说明从 Xcode 提取），
#            通过 --macos-sdk 指定；请自行确认 Apple SDK 的许可条款。未提供 SDK 时跳过 macOS。
#   all      linux + windows（提供 --macos-sdk 时再加 macos）
#
# 选项：
#   --engine docker|podman   容器引擎（默认自动探测）
#   --no-docker              不用容器，直接在本机执行同样的步骤（需本机已装好对应工具链，例如 mingw-w64）
#   --macos-sdk <路径>       macOS SDK 打包文件
#   --desktop-only           只构建桌面程序
#
# 本机直接构建 Windows 版（Debian/Ubuntu）：
#   sudo apt install gcc-mingw-w64-x86-64 && rustup target add x86_64-pc-windows-gnu
#   scripts/cross-build-docker.sh windows --no-docker
# shellcheck source=lib/common.sh
source "$(dirname "$0")/lib/common.sh"

WHAT="${1:-}"; if [[ -n "$WHAT" ]]; then shift; fi
ENGINE="" NO_DOCKER=0 MACOS_SDK="" DESKTOP_ONLY=0
while [[ $# -gt 0 ]]; do
    case "$1" in
        --engine) ENGINE="$2"; shift 2 ;;
        --no-docker) NO_DOCKER=1; shift ;;
        --macos-sdk) MACOS_SDK="$2"; shift 2 ;;
        --desktop-only) DESKTOP_ONLY=1; shift ;;
        -h|--help) sed -n '2,24p' "$0"; exit 0 ;;
        *) die "未知参数 $1" ;;
    esac
done
case "$WHAT" in linux|windows|macos|all) ;; *) sed -n '2,24p' "$0"; exit 1 ;; esac

PKGS=(-p labor-fee-desktop) BINS=(labor-fee-desktop)
if [[ $DESKTOP_ONLY -eq 0 ]]; then PKGS+=(-p labor-fee-cli -p labor-fee-web); BINS+=(labor-fee labor-fee-web); fi

# ---------- 构建步骤（容器内或 --no-docker 时在本机执行） ----------

build_windows_gnu() {
    local t=x86_64-pc-windows-gnu
    need x86_64-w64-mingw32-gcc "请安装 mingw-w64（Debian/Ubuntu：gcc-mingw-w64-x86-64）。"
    ensure_target "$t"
    export CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER=x86_64-w64-mingw32-gcc
    export CC_x86_64_pc_windows_gnu=x86_64-w64-mingw32-gcc CXX_x86_64_pc_windows_gnu=x86_64-w64-mingw32-g++
    export AR_x86_64_pc_windows_gnu=x86_64-w64-mingw32-ar
    log "构建 Windows（$t，mingw-w64）"
    (cd "$ROOT" && cargo build --release --locked "${PKGS[@]}" --target "$t")
    local tdir="${CARGO_TARGET_DIR:-$ROOT/target}"
    local out="$DIST/windows-gnu-x86_64"
    rm -rf "$out" && mkdir -p "$out"
    for b in "${BINS[@]}"; do cp "$tdir/$t/release/$b.exe" "$out/"; done
    copy_docs "$out"
    write_checksums "$out"
    local zip="$DIST/labor-fee-$VERSION-windows-gnu-x86_64.zip"
    rm -f "$zip"
    if command -v zip >/dev/null 2>&1; then
        (cd "$DIST" && zip -qr "$zip" "windows-gnu-x86_64") && log "压缩包：$zip"
    else
        log "未安装 zip，跳过生成压缩包"
    fi
    log "完成：$out"
    ls -l "$out"
}

build_linux() {
    local args=()
    [[ $DESKTOP_ONLY -eq 1 ]] && args+=(--desktop-only)
    "$ROOT/scripts/build-linux.sh" --target x86_64-unknown-linux-gnu "${args[@]}"
}

build_macos_osxcross() {
    [[ -n "$MACOS_SDK" && -f "$MACOS_SDK" ]] || { log "未提供 macOS SDK（--macos-sdk），跳过 macOS"; return 0; }
    local oc="${OSXCROSS_DIR:-$ROOT/target/osxcross}"
    if [[ ! -x "$oc/target/bin/oa64-clang" ]]; then
        log "构建 osxcross（首次较慢）"
        [[ -d "$oc/.git" ]] || git clone --depth 1 https://github.com/tpoechtrager/osxcross.git "$oc"
        cp "$MACOS_SDK" "$oc/tarballs/"
        (cd "$oc" && UNATTENDED=1 ./build.sh)
    fi
    export PATH="$oc/target/bin:$PATH"
    export MACOSX_DEPLOYMENT_TARGET="${MACOSX_DEPLOYMENT_TARGET:-11.0}"
    export CC_aarch64_apple_darwin=oa64-clang CXX_aarch64_apple_darwin=oa64-clang++
    export CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER=oa64-clang
    # 有 rcodesign 时 build-macos.sh 会自动 ad-hoc 签名，否则提示到 Mac 上再签
    local args=()
    [[ $DESKTOP_ONLY -eq 1 ]] && args+=(--desktop-only)
    "$ROOT/scripts/build-macos.sh" ${args[@]+"${args[@]}"}
}

run_inner() {
    case "$WHAT" in
        linux) build_linux ;;
        windows) build_windows_gnu ;;
        macos) build_macos_osxcross ;;
        all) build_linux; build_windows_gnu; build_macos_osxcross ;;
    esac
}

if [[ $NO_DOCKER -eq 1 || "${LABOR_FEE_IN_CONTAINER:-}" == 1 ]]; then
    run_inner
    exit 0
fi

# ---------- 容器 ----------
if [[ -z "$ENGINE" ]]; then
    if command -v docker >/dev/null 2>&1; then ENGINE=docker
    elif command -v podman >/dev/null 2>&1; then ENGINE=podman
    else die "没有找到 docker 或 podman。可安装其一，或加 --no-docker 在本机构建。"; fi
fi
IMAGE="labor-fee-cross:latest"
log "构建镜像 $IMAGE（$ENGINE）"
"$ENGINE" build -t "$IMAGE" -f "$ROOT/scripts/docker/Dockerfile.cross" "$ROOT/scripts/docker"

MOUNTS=(-v "$ROOT:/src" -v labor-fee-cargo-registry:/usr/local/cargo/registry)
EXTRA=()
if [[ -n "$MACOS_SDK" ]]; then
    MOUNTS+=(-v "$(cd "$(dirname "$MACOS_SDK")" && pwd)/$(basename "$MACOS_SDK"):/sdk/$(basename "$MACOS_SDK"):ro")
    EXTRA+=(--macos-sdk "/sdk/$(basename "$MACOS_SDK")")
fi
[[ $DESKTOP_ONLY -eq 1 ]] && EXTRA+=(--desktop-only)

log "在容器中构建：$WHAT"
"$ENGINE" run --rm "${MOUNTS[@]}" \
    -e LABOR_FEE_IN_CONTAINER=1 \
    -e CARGO_TARGET_DIR=/src/target/container \
    -e OSXCROSS_DIR=/src/target/osxcross \
    -w /src "$IMAGE" \
    bash -c "scripts/cross-build-docker.sh $WHAT ${EXTRA[*]} && chown -R $(id -u):$(id -g) /src/dist /src/target/container 2>/dev/null || true"
log "产物见 $DIST"
