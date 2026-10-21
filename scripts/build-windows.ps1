<#
.SYNOPSIS
  Windows 本机构建：默认 MSVC 工具链，生成 dist\windows-<arch>\labor-fee-desktop.exe 等及 zip。

.DESCRIPTION
  依赖：
    - Rust 1.95+（https://rustup.rs，默认 stable-x86_64-pc-windows-msvc）
    - Visual Studio 2019+ 生成工具（“使用 C++ 的桌面开发”工作负载，提供 link.exe 与 Windows SDK）
  桌面程序发布构建为 Windows 子系统（源码中 windows_subsystem = "windows"），双击运行不会弹出控制台窗口。
  不需要 GTK；文件对话框使用系统原生对话框。

.PARAMETER Target
  Rust 目标，默认 x86_64-pc-windows-msvc；也可 aarch64-pc-windows-msvc 或 x86_64-pc-windows-gnu。

.PARAMETER DesktopOnly
  只构建桌面程序（默认同时构建命令行 labor-fee.exe 与 labor-fee-web.exe）。

.EXAMPLE
  powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1
#>
param(
    [string]$Target = "x86_64-pc-windows-msvc",
    [switch]$DesktopOnly
)
$ErrorActionPreference = "Stop"
$Root = Resolve-Path (Join-Path $PSScriptRoot "..")
$Dist = Join-Path $Root "dist"
$Version = (Select-String -Path (Join-Path $Root "Cargo.toml") -Pattern '^version = "(.*)"' | Select-Object -First 1).Matches[0].Groups[1].Value

function Need($cmd, $hint) {
    if (-not (Get-Command $cmd -ErrorAction SilentlyContinue)) { throw "缺少 $cmd。$hint" }
}
Need cargo "请先安装 Rust：https://rustup.rs"
if ($Target -like "*msvc*" -and -not (Get-Command link.exe -ErrorAction SilentlyContinue)) {
    # rustup 会自动定位 VS 生成工具；这里只提示
    Write-Warning "当前 PATH 中没有 link.exe；如构建报链接器错误，请安装 Visual Studio 生成工具（C++ 桌面开发）。"
}
if (Get-Command rustup -ErrorAction SilentlyContinue) {
    if (-not ((rustup target list --installed) -contains $Target)) { rustup target add $Target }
}

$Arch = $Target.Split("-")[0]
Push-Location $Root
try {
    Write-Host "==> 构建桌面程序（$Target）"
    cargo build --release --locked -p labor-fee-desktop --target $Target
    if ($LASTEXITCODE -ne 0) { throw "构建失败" }
    $Bins = @("labor-fee-desktop")
    if (-not $DesktopOnly) {
        Write-Host "==> 构建命令行与 Web 服务"
        cargo build --release --locked -p labor-fee-cli -p labor-fee-web --target $Target
        if ($LASTEXITCODE -ne 0) { throw "构建失败" }
        $Bins += @("labor-fee", "labor-fee-web")
    }
    $Out = Join-Path $Dist "windows-$Arch"
    if (Test-Path $Out) { Remove-Item -Recurse -Force $Out }
    New-Item -ItemType Directory -Force $Out | Out-Null
    foreach ($b in $Bins) { Copy-Item "target\$Target\release\$b.exe" $Out }
    Copy-Item "crates\desktop\assets\fonts\OFL.txt" (Join-Path $Out "字体许可证-OFL.txt")
    @"
劳务报酬计算工具 $Version

labor-fee-desktop.exe  桌面程序（双击运行）
labor-fee.exe          命令行工具（如随包提供）
labor-fee-web.exe      Web 服务（如随包提供）

自定义口径：labor-fee-desktop.exe --config 口径.toml，或在程序“口径”页签加载。
界面内嵌字体为 Noto Sans CJK SC 子集（SIL OFL 1.1），见 字体许可证-OFL.txt。
"@ | Set-Content -Encoding UTF8 (Join-Path $Out "说明.txt")
    Get-ChildItem $Out -File | Get-FileHash -Algorithm SHA256 |
        ForEach-Object { "{0}  {1}" -f $_.Hash.ToLower(), (Split-Path $_.Path -Leaf) } |
        Set-Content -Encoding ASCII (Join-Path $Out "SHA256SUMS")
    $Zip = Join-Path $Dist "labor-fee-$Version-windows-$Arch.zip"
    if (Test-Path $Zip) { Remove-Item $Zip }
    Compress-Archive -Path $Out -DestinationPath $Zip
    Write-Host "==> 完成：$Out"
    Write-Host "==> 压缩包：$Zip"
} finally {
    Pop-Location
}
