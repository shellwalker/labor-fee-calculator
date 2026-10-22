use crate::error::CalcError;
use rust_decimal::{Decimal, RoundingStrategy};
use serde::de::IgnoredAny;
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

/// 增值税附加税费的舍入时点。
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

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct VatConfig {
    /// 增值税征收率：增值税税额 = round(不含增值税收入 × 征收率)。
    pub rate: Decimal,
    /// 是否启用起征点：不含增值税收入未达到起征点时，增值税及附加税费均为 0。
    pub threshold_enabled: bool,
    /// 起征点金额（按不含增值税收入判断）。
    pub threshold: Decimal,
    /// 不含增值税收入恰好等于起征点时是否征收（true：达到即征收；false：超过才征收）。
    pub levy_at_threshold: bool,
}

/// 增值税附加税费的计税依据。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SurchargeBase {
    /// 以实际缴纳的增值税税额为计税依据（法定口径）。
    Vat,
    /// 以不含增值税收入为计税依据（非法定口径，仅供测算）。
    NetRevenue,
}

/// 城市维护建设税。
pub const URBAN_CONSTRUCTION_TAX: &str = "城市维护建设税";
/// 教育费附加。
pub const EDUCATION_SURCHARGE: &str = "教育费附加";
/// 地方教育附加。
pub const LOCAL_EDUCATION_SURCHARGE: &str = "地方教育附加";
/// 旧版配置中的合并项目名称，读入时视为教育费附加。
pub const LEGACY_SURCHARGE_NAME: &str = "附加税";

fn yes() -> bool {
    true
}

/// 增值税附加税费的一个项目（如城市维护建设税、教育费附加、地方教育附加）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SurchargeItem {
    pub name: String,
    /// 税（费）率，以计税依据为基数。
    pub rate: Decimal,
    /// 是否计征；未启用的项目不参与计算，也不在结果中列示。
    #[serde(default = "yes")]
    pub enabled: bool,
}

impl SurchargeItem {
    pub fn new(name: &str, rate: Decimal, enabled: bool) -> Self {
        SurchargeItem {
            name: name.to_string(),
            rate,
            enabled,
        }
    }

    /// 标准三项：城市维护建设税、教育费附加、地方教育附加。
    /// 教育费附加按 `education` 计征且启用；城市维护建设税（预填 7%）与地方教育附加（预填 2%）不启用。
    pub fn standard(education: Decimal) -> Vec<SurchargeItem> {
        vec![
            SurchargeItem::new(URBAN_CONSTRUCTION_TAX, Decimal::new(7, 2), false),
            SurchargeItem::new(EDUCATION_SURCHARGE, education, true),
            SurchargeItem::new(LOCAL_EDUCATION_SURCHARGE, Decimal::new(2, 2), false),
        ]
    }
}

/// 增值税附加税费配置。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "SurchargeConfigRaw")]
pub struct SurchargeConfig {
    pub base: SurchargeBase,
    pub items: Vec<SurchargeItem>,
}

/// 读取用的原始结构，兼容旧版字段：
/// - `halve`、`relief_enabled`、`relief_rate`（减征，已不再支持）→ 忽略；
/// - 单一 `rate`（未列 `items`）或名为“附加税”的项目 → 教育费附加，并补齐另外两项（未启用）。
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SurchargeConfigRaw {
    base: SurchargeBase,
    #[serde(default, rename = "relief_enabled")]
    _relief_enabled: Option<IgnoredAny>,
    #[serde(default, rename = "relief_rate")]
    _relief_rate: Option<IgnoredAny>,
    #[serde(default, rename = "halve")]
    _halve: Option<IgnoredAny>,
    #[serde(default)]
    rate: Option<Decimal>,
    #[serde(default)]
    items: Vec<SurchargeItem>,
}

impl From<SurchargeConfigRaw> for SurchargeConfig {
    fn from(r: SurchargeConfigRaw) -> Self {
        let mut items = r.items;
        if items.is_empty() {
            if let Some(rate) = r.rate {
                items = SurchargeItem::standard(rate);
            }
        } else if items.len() == 1 && items[0].name == LEGACY_SURCHARGE_NAME {
            let it = &items[0];
            let mut std = SurchargeItem::standard(it.rate);
            std[1].enabled = it.enabled;
            items = std;
        } else {
            for it in &mut items {
                if it.name == LEGACY_SURCHARGE_NAME {
                    it.name = EDUCATION_SURCHARGE.to_string();
                }
            }
        }
        SurchargeConfig {
            base: r.base,
            items,
        }
    }
}

impl SurchargeConfig {
    /// 已启用的项目。
    pub fn enabled_items(&self) -> impl Iterator<Item = &SurchargeItem> {
        self.items.iter().filter(|i| i.enabled)
    }
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
    pub fixed_deduction_limit: Decimal,
    pub fixed_deduction: Decimal,
    pub percent_deduction_rate: Decimal,
    pub brackets: Vec<Bracket>,
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
    /// 多个不含增值税收入得到同一税后实发金额时取最小（默认）或最大。
    pub prefer: Prefer,
    /// 二分定位后前后逐分扫描的范围（分）。
    pub search_window: u32,
    /// 测算时不含增值税收入的搜索上限。
    pub max_net_income: Decimal,
}

/// 全部计算参数与舍入规则。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub rounding: RoundingConfig,
    pub vat: VatConfig,
    pub surcharge: SurchargeConfig,
    pub iit: IitConfig,
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
            if it.name.trim().is_empty() {
                return bad("surcharge.items.name 不能为空");
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
        if self.reverse.max_net_income <= Decimal::ZERO {
            return bad("reverse.max_net_income 必须为正");
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

    /// 已启用的附加税费各项目的费率，与 [`SurchargeConfig::enabled_items`] 顺序一致。
    pub fn effective_surcharge_rates(&self) -> Vec<Decimal> {
        self.surcharge.enabled_items().map(|it| it.rate).collect()
    }

    /// 不含增值税收入为 `amount` 时是否征收增值税及附加税费。
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
        let on: Vec<_> = cfg.surcharge.enabled_items().collect();
        assert_eq!(on.len(), 1);
        assert_eq!(on[0].name, EDUCATION_SURCHARGE);
        assert_eq!(on[0].rate, Decimal::new(3, 2));
        assert_eq!(cfg.surcharge.items.len(), 3);
        assert_eq!(cfg.surcharge.items[0].rate, Decimal::new(7, 2));
        assert!(!cfg.surcharge.items[0].enabled);
        assert_eq!(cfg.surcharge.items[2].rate, Decimal::new(2, 2));
        assert!(!cfg.surcharge.items[2].enabled);
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

    fn legacy(surcharge: &str) -> Config {
        let start = DEFAULT_CONFIG_TOML.find("[surcharge]").unwrap();
        let end = DEFAULT_CONFIG_TOML.find("[iit]").unwrap();
        let s = format!(
            "{}{}\n{}",
            &DEFAULT_CONFIG_TOML[..start],
            surcharge,
            &DEFAULT_CONFIG_TOML[end..]
        );
        Config::from_toml_str(&s).unwrap()
    }

    #[test]
    fn legacy_single_surcharge_is_education() {
        let def = Config::default();
        // 旧版：单一“附加税”项目
        let a = legacy(
            "[surcharge]\nbase = \"vat\"\nhalve = false\n[[surcharge.items]]\nname = \"附加税\"\nrate = \"0.03\"\n",
        );
        assert_eq!(a, def);
        // 旧版：单一 rate 字段
        let b = legacy("[surcharge]\nbase = \"vat\"\nrate = \"0.03\"\n");
        assert_eq!(b, def);
    }

    #[test]
    fn legacy_relief_fields_ignored() {
        // 减征已不再支持：旧版 halve / relief_* 字段读入时忽略，不报错、不影响费率
        let def = Config::default();
        let a = legacy(
            "[surcharge]\nbase = \"vat\"\nhalve = true\n[[surcharge.items]]\nname = \"附加税\"\nrate = \"0.03\"\n",
        );
        assert_eq!(a, def);
        assert_eq!(a.effective_surcharge_rates(), vec![Decimal::new(3, 2)]);
        let mut s = DEFAULT_CONFIG_TOML.replace(
            "base = \"vat\"",
            "base = \"vat\"\nrelief_enabled = true\nrelief_rate = \"0.5\"",
        );
        let b = Config::from_toml_str(&s).unwrap();
        assert_eq!(b, def);
        s = s.replace("relief_rate = \"0.5\"", "relief_rate = \"9\"");
        assert_eq!(Config::from_toml_str(&s).unwrap(), def);
        // 重新保存时不再写出这些字段
        assert!(!b.to_toml_string().contains("relief"));
    }

    #[test]
    fn unknown_field_rejected() {
        let s = DEFAULT_CONFIG_TOML.replace("[vat]", "[vat]\nfoo = 1");
        assert!(Config::from_toml_str(&s).is_err());
    }
}
