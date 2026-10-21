use crate::error::CalcError;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::{Deserialize, Serialize};

/// 内置默认配置（与仓库 `config/default.toml` 一致）。
pub const DEFAULT_CONFIG_TOML: &str = include_str!("../../../config/default.toml");

/// 舍入方式。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RoundingMode {
    /// 四舍五入（0.5 远离零），即 MidpointAwayFromZero。
    HalfUp,
    /// 银行家舍入（0.5 取偶）。
    HalfEven,
    /// 截断（向零）。
    Down,
    /// 进位（远离零）。
    Up,
}

impl RoundingMode {
    fn strategy(self) -> RoundingStrategy {
        match self {
            RoundingMode::HalfUp => RoundingStrategy::MidpointAwayFromZero,
            RoundingMode::HalfEven => RoundingStrategy::MidpointNearestEven,
            RoundingMode::Down => RoundingStrategy::ToZero,
            RoundingMode::Up => RoundingStrategy::AwayFromZero,
        }
    }
}

/// 附加税的舍入时点。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurchargeRounding {
    /// 每个税目分别舍入后相加。
    PerItem,
    /// 各税目先不舍入，合计后一次舍入（明细按比例展示，可能与合计差 1 分）。
    Total,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoundingConfig {
    pub scale: u32,
    pub mode: RoundingMode,
    pub surcharge: SurchargeRounding,
}

/// 企业总成本口径（假设 A）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CostBasis {
    /// 总成本 = 价税合计 = 不含税收入 + 增值税。
    VatInclusive,
    /// 总成本 = 不含税收入 + 增值税 + 附加税。
    VatAndSurchargeInclusive,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CostConfig {
    pub basis: CostBasis,
    /// 总成本含全部税费时，倒挤出的收入再算增值税与原值不等，是否在相邻一分钱内寻找自洽组合。
    pub consistent_search: bool,
}

/// 门槛跳变段的处理方式（总成本按不征收时收入已达门槛，按征收倒挤后收入又低于门槛）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdGap {
    /// 按征收处理：倒挤出的不含税收入可能略低于门槛。
    Levy,
    /// 按不征收处理：不含税收入 = 总成本（已达到或超过门槛）。
    Exempt,
}

/// 增值税/附加税征收门槛的判断基数。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ThresholdBasis {
    /// 按不含税开票收入判断。
    NetRevenue,
    /// 按企业总成本判断（不征收时总成本 = 不含税收入）。
    TotalCost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VatConfig {
    /// 增值税征收率。
    pub rate: Decimal,
    /// 是否启用征收门槛：低于门槛时增值税、附加税均为 0。
    pub threshold_enabled: bool,
    /// 门槛金额。
    pub threshold: Decimal,
    /// 门槛判断基数。
    pub threshold_basis: ThresholdBasis,
    /// 金额恰好等于门槛时是否征收（true：≥ 门槛征收；false：> 门槛才征收）。
    pub levy_at_threshold: bool,
    /// 门槛跳变段的处理方式（仅门槛按不含税收入判断时有意义）。
    pub threshold_gap: ThresholdGap,
}

/// 附加税计税基数（假设 B）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurchargeBase {
    /// 以实际缴纳的增值税额为基数（法定口径）。
    Vat,
    /// 以不含税开票收入为基数。
    NetRevenue,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurchargeItem {
    pub name: String,
    pub rate: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurchargeConfig {
    pub base: SurchargeBase,
    pub halve: bool,
    pub items: Vec<SurchargeItem>,
}

/// 个税收入额基数（假设 C）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum IncomeBase {
    /// 不含税收入（价税分离后）。
    NetRevenue,
    /// 企业总成本。
    TotalCost,
}

/// 预扣率表一档。`upper` 为应纳税所得额上限（含本数），最后一档为 `None`。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Bracket {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub upper: Option<Decimal>,
    pub rate: Decimal,
    pub quick_deduction: Decimal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IitConfig {
    pub income_base: IncomeBase,
    pub deduct_surcharge: bool,
    pub fixed_deduction_limit: Decimal,
    pub fixed_deduction: Decimal,
    pub percent_deduction_rate: Decimal,
    pub brackets: Vec<Bracket>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TakeHomeConfig {
    pub deduct_surcharge: bool,
}

/// 多解时的取值偏好。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Prefer {
    MinCost,
    MaxCost,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ReverseConfig {
    pub prefer: Prefer,
    pub search_window: u32,
    pub max_cost: Decimal,
}

/// 全部税务口径与舍入规则。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub rounding: RoundingConfig,
    pub cost: CostConfig,
    pub vat: VatConfig,
    pub surcharge: SurchargeConfig,
    pub iit: IitConfig,
    pub take_home: TakeHomeConfig,
    pub reverse: ReverseConfig,
}

impl Default for Config {
    fn default() -> Self {
        Config::from_toml_str(DEFAULT_CONFIG_TOML).expect("内置默认配置必须有效")
    }
}

impl Config {
    /// 从 TOML 文本解析并校验配置。
    pub fn from_toml_str(s: &str) -> Result<Self, CalcError> {
        let cfg: Config = toml::from_str(s).map_err(|e| CalcError::InvalidConfig(e.to_string()))?;
        cfg.validate()?;
        Ok(cfg)
    }

    /// 序列化为 TOML 文本。
    pub fn to_toml_string(&self) -> String {
        toml::to_string_pretty(self).expect("配置可序列化")
    }

    /// 校验配置的合理性。
    pub fn validate(&self) -> Result<(), CalcError> {
        let bad = |m: &str| Err(CalcError::InvalidConfig(m.to_string()));
        let one = Decimal::ONE;
        if self.rounding.scale > 6 {
            return bad("rounding.scale 不能超过 6");
        }
        if self.vat.rate < Decimal::ZERO || self.vat.rate >= one {
            return bad("vat.rate 必须在 [0, 1) 内");
        }
        if self.vat.threshold < Decimal::ZERO {
            return bad("vat.threshold 不能为负");
        }
        for it in &self.surcharge.items {
            if it.rate < Decimal::ZERO || it.rate >= one {
                return bad("surcharge.items.rate 必须在 [0, 1) 内");
            }
        }
        let iit = &self.iit;
        if iit.fixed_deduction < Decimal::ZERO || iit.fixed_deduction_limit < Decimal::ZERO {
            return bad("iit 减除费用参数不能为负");
        }
        if iit.percent_deduction_rate < Decimal::ZERO || iit.percent_deduction_rate >= one {
            return bad("iit.percent_deduction_rate 必须在 [0, 1) 内");
        }
        if iit.brackets.is_empty() {
            return bad("iit.brackets 不能为空");
        }
        let mut prev: Option<Decimal> = None;
        for (i, b) in iit.brackets.iter().enumerate() {
            let last = i + 1 == iit.brackets.len();
            if b.rate < Decimal::ZERO || b.rate >= one || b.quick_deduction < Decimal::ZERO {
                return bad("iit.brackets 的 rate 必须在 [0,1) 内，quick_deduction 不能为负");
            }
            match (b.upper, last) {
                (None, true) => {}
                (None, false) => return bad("只有最后一档预扣率可以省略 upper"),
                (Some(_), true) => return bad("最后一档预扣率不能设置 upper"),
                (Some(u), false) => {
                    if prev.is_some_and(|p| u <= p) {
                        return bad("iit.brackets 的 upper 必须严格递增");
                    }
                    prev = Some(u);
                }
            }
        }
        if self.reverse.max_cost <= Decimal::ZERO {
            return bad("reverse.max_cost 必须为正");
        }
        Ok(())
    }

    /// 按配置舍入到最小金额单位。
    pub fn round(&self, d: Decimal) -> Decimal {
        d.round_dp_with_strategy(self.rounding.scale, self.rounding.mode.strategy())
    }

    /// 最小金额单位（scale=2 时为 0.01）。
    pub fn unit(&self) -> Decimal {
        Decimal::new(1, self.rounding.scale)
    }

    /// 附加税各税目的实际税率（考虑减半）。
    pub fn effective_surcharge_rates(&self) -> Vec<Decimal> {
        let half = Decimal::new(5, 1);
        self.surcharge
            .items
            .iter()
            .map(|it| {
                if self.surcharge.halve {
                    it.rate * half
                } else {
                    it.rate
                }
            })
            .collect()
    }

    /// 门槛判断金额为 `amount` 时是否征收增值税和附加税。
    pub fn levied(&self, amount: Decimal) -> bool {
        if !self.vat.threshold_enabled {
            return true;
        }
        if self.vat.levy_at_threshold {
            amount >= self.vat.threshold
        } else {
            amount > self.vat.threshold
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_parses_and_roundtrips() {
        let cfg = Config::default();
        assert_eq!(cfg.vat.rate, Decimal::new(1, 2));
        assert_eq!(cfg.surcharge.items[0].rate, Decimal::new(3, 2));
        let again = Config::from_toml_str(&cfg.to_toml_string()).unwrap();
        assert_eq!(cfg, again);
    }

    #[test]
    fn invalid_brackets_rejected() {
        let mut cfg = Config::default();
        cfg.iit.brackets[1].upper = Some(Decimal::new(100, 0));
        assert!(cfg.validate().is_err());
        let mut cfg = Config::default();
        cfg.iit.brackets.last_mut().unwrap().upper = Some(Decimal::new(1_000_000, 0));
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn unknown_field_rejected() {
        let s = DEFAULT_CONFIG_TOML.replace("[vat]", "[vat]\nfoo = 1");
        assert!(Config::from_toml_str(&s).is_err());
    }
}
