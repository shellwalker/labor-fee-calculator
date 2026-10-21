//! 劳务报酬计算核心库。
//!
//! - 正算：企业总成本 → 不含税开票收入、增值税、附加税、劳务报酬个税、实际到手。
//! - 反算：实际到手 → 企业总成本（含分位校准，保证正算回去与目标完全一致或明确标记不可精确达到）。
//!
//! 所有金额使用 [`rust_decimal::Decimal`]，不使用浮点数。税务口径与舍入规则见 [`Config`]。

mod config;
mod error;
mod forward;
mod reverse;

pub use config::{
    Bracket, Config, CostBasis, CostConfig, IitConfig, IncomeBase, Prefer, ReverseConfig,
    RoundingConfig, RoundingMode, SurchargeBase, SurchargeConfig, SurchargeItem, SurchargeRounding,
    TakeHomeConfig, ThresholdBasis, ThresholdGap, VatConfig, DEFAULT_CONFIG_TOML,
};
pub use error::CalcError;
pub use forward::{forward, Breakdown, IitBreakdown, SurchargeLine};
pub use reverse::{reverse, ReverseOutcome, Reverser};

pub use rust_decimal::Decimal;
