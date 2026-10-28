//! 批量测算的持久化：各测算方式的导出列设置（选择与顺序）、上次的输入文件与列标题，
//! 以及上次计算所用输入文件的副本（用于下次启动时恢复结果）。
//!
//! 保存在设置目录（见 [`crate::settings`]）下：
//! - `batch.toml`：当前测算方式；每种方式的输入文件路径、列标题、导出列顺序与不导出的列；
//! - `batch-cache/<方式>.<扩展名>`：上次成功计算时输入文件的副本（不超过 [`CACHE_LIMIT`]）。
//!
//! 下次启动时按**当前参数**用副本重新计算并显示结果，原文件被移动或删除也能恢复。

use crate::settings::SettingsStore;
use labor_fee_batch::{ColumnLayout, InputKind};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 输入文件副本的大小上限（字节）；超过时不缓存，下次启动只恢复路径与导出列设置。
pub const CACHE_LIMIT: usize = 32 * 1024 * 1024;

/// 三种测算方式（与界面下拉框顺序一致）。
pub const KINDS: [InputKind; 3] = [
    InputKind::TakeHome,
    InputKind::NetIncome,
    InputKind::TotalCost,
];

/// 测算方式在数组中的下标。
pub fn kind_index(k: InputKind) -> usize {
    match k {
        InputKind::TakeHome => 0,
        InputKind::NetIncome => 1,
        InputKind::TotalCost => 2,
    }
}

/// 测算方式的英文标识（文件名与 `batch.toml` 使用）。
pub fn kind_key(k: InputKind) -> &'static str {
    match k {
        InputKind::TakeHome => "take-home",
        InputKind::NetIncome => "net-income",
        InputKind::TotalCost => "total-cost",
    }
}

/// 一种测算方式的保存内容。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ModePrefs {
    /// 输入文件路径。
    pub path: String,
    /// 读取的列标题（空为默认）。
    pub column: String,
    /// 导出列顺序（列标识）；空为默认顺序。
    pub order: Vec<String>,
    /// 不导出的列（列标识）。
    pub hidden: Vec<String>,
    /// 输入文件副本（`batch-cache/` 下的文件名）；没有可恢复的结果时为空。
    pub cached: String,
    /// 副本对应的原文件路径（恢复后显示与默认保存位置用）。
    pub cached_from: String,
}

impl ModePrefs {
    pub fn layout(&self) -> ColumnLayout {
        ColumnLayout {
            order: self.order.clone(),
            hidden: self.hidden.clone(),
        }
    }

    pub fn set_layout(&mut self, l: &ColumnLayout) {
        self.order = l.order.clone();
        self.hidden = l.hidden.clone();
    }
}

/// `batch.toml` 的内容。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct BatchPrefs {
    /// 当前测算方式（take-home / net-income / total-cost）。
    pub kind: String,
    pub take_home: ModePrefs,
    pub net_income: ModePrefs,
    pub total_cost: ModePrefs,
}

impl BatchPrefs {
    pub fn kind(&self) -> InputKind {
        InputKind::parse(&self.kind).unwrap_or_default()
    }

    pub fn mode(&self, k: InputKind) -> &ModePrefs {
        match k {
            InputKind::TakeHome => &self.take_home,
            InputKind::NetIncome => &self.net_income,
            InputKind::TotalCost => &self.total_cost,
        }
    }

    pub fn mode_mut(&mut self, k: InputKind) -> &mut ModePrefs {
        match k {
            InputKind::TakeHome => &mut self.take_home,
            InputKind::NetIncome => &mut self.net_income,
            InputKind::TotalCost => &mut self.total_cost,
        }
    }
}

impl SettingsStore {
    pub fn batch_path(&self) -> PathBuf {
        self.dir.join("batch.toml")
    }

    pub fn batch_cache_dir(&self) -> PathBuf {
        self.dir.join("batch-cache")
    }

    /// 读取批量测算设置；文件不存在或损坏时用默认值。
    pub fn load_batch(&self) -> BatchPrefs {
        std::fs::read_to_string(self.batch_path())
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save_batch(&self, p: &BatchPrefs) -> Result<(), String> {
        std::fs::create_dir_all(&self.dir).map_err(|e| format!("创建设置目录失败：{e}"))?;
        let text = toml::to_string_pretty(p).map_err(|e| e.to_string())?;
        let path = self.batch_path();
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text).map_err(|e| format!("保存批量测算设置失败：{e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("保存批量测算设置失败：{e}"))
    }

    /// 保存输入文件副本，返回文件名；超过大小上限时删除旧副本并返回 None。
    pub fn save_batch_input(
        &self,
        kind: InputKind,
        input: &Path,
        bytes: &[u8],
    ) -> Result<Option<String>, String> {
        let dir = self.batch_cache_dir();
        self.remove_batch_input(kind);
        if bytes.len() > CACHE_LIMIT {
            return Ok(None);
        }
        std::fs::create_dir_all(&dir).map_err(|e| format!("创建缓存目录失败：{e}"))?;
        let ext = input
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_else(|| "xlsx".into());
        let name = format!("{}.{ext}", kind_key(kind));
        let path = dir.join(&name);
        let tmp = dir.join(format!("{name}.tmp"));
        std::fs::write(&tmp, bytes).map_err(|e| format!("缓存输入文件失败：{e}"))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("缓存输入文件失败：{e}"))?;
        Ok(Some(name))
    }

    /// 读取输入文件副本。
    pub fn load_batch_input(&self, name: &str) -> Option<Vec<u8>> {
        if name.is_empty() || name.contains(['/', '\\']) {
            return None;
        }
        std::fs::read(self.batch_cache_dir().join(name)).ok()
    }

    /// 删除某种方式的输入文件副本。
    pub fn remove_batch_input(&self, kind: InputKind) {
        let Ok(rd) = std::fs::read_dir(self.batch_cache_dir()) else {
            return;
        };
        let prefix = format!("{}.", kind_key(kind));
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}
