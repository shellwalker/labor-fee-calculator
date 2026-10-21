//! 劳务报酬计算核心库。
//!
//! 核心变量是 **不含税收入**（未计增值税的劳务报酬收入）：
//! - 正算：不含税收入 → 增值税、附加税、企业总成本、劳务报酬个税、实际到手，每一项都由它唯一确定；
//! - 反算：实际到手 → 不含税收入（分位校准，保证正算回去与目标完全一致，或明确标记不可精确达到）；
//! - 辅助：企业总成本 → 不含税收入（总成本没有对应收入时报错并给出最近的两个可达总成本）。
//!
//! 所有金额使用 [`rust_decimal::Decimal`]，不使用浮点数。税务口径与舍入规则见 [`Config`]。

mod config;
mod error;
mod forward;
mod reverse;

pub use config::{
    Bracket, Config, IitConfig, Prefer, ReverseConfig, RoundingConfig, RoundingMode, SurchargeBase,
    SurchargeConfig, SurchargeItem, SurchargeRounding, VatConfig, DEFAULT_CONFIG_TOML,
};
pub use error::{CalcError, CostGap, CostGapReason, ReachableCost};
pub use forward::{forward, from_total_cost, Breakdown, IitBreakdown, SurchargeLine};
pub use reverse::{reverse, ReverseOutcome, Reverser};

pub use rust_decimal::Decimal;
