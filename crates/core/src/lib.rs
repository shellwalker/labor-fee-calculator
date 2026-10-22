//! 劳务报酬计算核心库。
//!
//! 核心变量是 **不含增值税收入**（劳务报酬的每次收入，不含增值税）：
//! - 按收入计算：不含增值税收入 → 增值税税额、增值税附加税费、企业总支出、应预扣预缴税额、税后实发金额，
//!   每一项都由它唯一确定；
//! - 按税后实发金额测算：税后实发金额 → 不含增值税收入（逐分校准，保证再计算与目标完全一致，或明确标记无法精确匹配）；
//! - 按企业总支出测算：企业总支出 → 不含增值税收入（无对应收入时报错，并给出两侧最近的有效总支出）。
//!
//! 所有金额使用 [`rust_decimal::Decimal`]，不使用浮点数。税务口径与舍入规则见 [`Config`]。

mod config;
mod error;
mod forward;
mod reverse;

pub use config::{
    Bracket, Config, IitConfig, Prefer, ReverseConfig, RoundingConfig, RoundingMode, SurchargeBase,
    SurchargeConfig, SurchargeItem, SurchargeRounding, UrbanConstructionConfig, UrbanRegion,
    VatConfig, DEFAULT_CONFIG_TOML, DEFAULT_RELIEF_RATE, DEFAULT_URBAN_REGION, EDUCATION_SURCHARGE,
    LEGACY_SURCHARGE_NAME, LOCAL_EDUCATION_SURCHARGE, URBAN_CONSTRUCTION_RATES,
    URBAN_CONSTRUCTION_TAX,
};
pub use error::{CalcError, CostGap, CostGapReason, ReachableCost};
pub use forward::{forward, from_total_cost, Breakdown, IitBreakdown, SurchargeLine};
pub use reverse::{reverse, ReverseOutcome, Reverser};

pub use rust_decimal::Decimal;
