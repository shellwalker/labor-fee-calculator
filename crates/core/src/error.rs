use rust_decimal::Decimal;
use serde::Serialize;
use thiserror::Error;

/// 企业总成本没有对应不含税收入的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CostGapReason {
    /// 增值税征收门槛造成的空白区间。
    Threshold,
    /// 增值税、附加税逐分舍入造成的单分空位。
    Rounding,
}

/// 一个可达的企业总成本及其对应的不含税收入。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct ReachableCost {
    pub net_income: Decimal,
    pub total_cost: Decimal,
}

/// 总成本空位的详细信息。
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CostGap {
    /// 输入的企业总成本。
    pub total_cost: Decimal,
    pub reason: CostGapReason,
    /// 下侧最近的可达总成本。
    pub lower: ReachableCost,
    /// 上侧最近的可达总成本。
    pub upper: ReachableCost,
    /// 门槛金额（原因为门槛时）。
    #[serde(skip_serializing_if = "Option::is_none")]
    pub threshold: Option<Decimal>,
}

impl std::fmt::Display for CostGap {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let (l, u) = (self.lower, self.upper);
        match self.reason {
            CostGapReason::Threshold => write!(
                f,
                "企业总成本 {} 没有对应的不含税收入：该总成本处于 {} 元增值税征收门槛形成的空白区间——不含税收入 {} 对应总成本 {}，{} 对应 {}，这一段没有对应的收入。两侧最近的有效总成本：{}（不含税收入 {}）、{}（不含税收入 {}）",
                self.total_cost,
                self.threshold.map(|t| t.normalize().to_string()).unwrap_or_default(),
                l.net_income, l.total_cost, u.net_income, u.total_cost,
                l.total_cost, l.net_income, u.total_cost, u.net_income
            ),
            CostGapReason::Rounding => write!(
                f,
                "企业总成本 {} 没有对应的不含税收入：原因是增值税、附加税逐分舍入——不含税收入 {} 对应总成本 {}，{} 对应 {}，中间的总成本取不到。两侧最近的有效总成本：{}（不含税收入 {}）、{}（不含税收入 {}）",
                self.total_cost,
                l.net_income, l.total_cost, u.net_income, u.total_cost,
                l.total_cost, l.net_income, u.total_cost, u.net_income
            ),
        }
    }
}

/// 计算与配置错误。
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CalcError {
    #[error("金额不能为负数：{0}")]
    NegativeAmount(Decimal),
    #[error("金额 {amount} 的小数位超过配置的精度（{scale} 位）")]
    PrecisionExceeded { amount: Decimal, scale: u32 },
    #[error("目标到手金额 {target} 超出可计算范围（不含税收入上限 {max_net_income} 时到手仅 {max_take_home}）")]
    TargetTooLarge {
        target: Decimal,
        max_net_income: Decimal,
        max_take_home: Decimal,
    },
    #[error("{0}")]
    CostUnreachable(Box<CostGap>),
    #[error("配置无效：{0}")]
    InvalidConfig(String),
    #[error("金额计算溢出")]
    Overflow,
}
