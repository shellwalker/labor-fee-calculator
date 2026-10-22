//! 构建脚本：
//! - 写入构建信息（git 短哈希、构建日期），供“关于”窗口和状态栏显示；
//! - Windows 目标嵌入可执行文件图标（MSVC 用 rc.exe，mingw 用 windres）。
//!
//! 可用环境变量覆盖：`LFD_GIT_HASH`、`SOURCE_DATE_EPOCH`（可复现构建）。

use std::process::Command;

fn git_hash() -> String {
    if let Ok(h) = std::env::var("LFD_GIT_HASH") {
        return h;
    }
    Command::new("git")
        .args(["rev-parse", "--short=8", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".into())
}

/// 自 1970-01-01 起的天数 → (年, 月, 日)（UTC，Howard Hinnant 算法）。
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn build_date() -> String {
    let secs = std::env::var("SOURCE_DATE_EPOCH")
        .ok()
        .and_then(|s| s.parse::<i64>().ok())
        .unwrap_or_else(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0)
        });
    let (y, m, d) = civil_from_days(secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

fn main() {
    println!("cargo:rustc-env=LFD_GIT_HASH={}", git_hash());
    println!("cargo:rustc-env=LFD_BUILD_DATE={}", build_date());
    println!("cargo:rerun-if-env-changed=LFD_GIT_HASH");
    println!("cargo:rerun-if-env-changed=SOURCE_DATE_EPOCH");
    for p in [
        "../../.git/HEAD",
        "../../.git/refs/heads",
        "build.rs",
        "app.rc",
        "assets/icon/icon.ico",
    ] {
        println!("cargo:rerun-if-changed={p}");
    }
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_resource::compile("app.rc", embed_resource::NONE)
            .manifest_optional()
            .expect("嵌入 Windows 图标资源失败（需要 rc.exe 或 windres）");
    }
}
