use rust_decimal::Decimal;
use serde::Serialize;
use thiserror::Error;

/// 企业总支出无对应不含增值税收入的原因。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CostGapReason {
    /// 增值税起征点形成的不可达区间。
    Threshold,
    /// 增值税税额及附加税费按分舍入形成的不可达金额。
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
                "企业总支出 {} 无对应的不含增值税收入：该金额位于增值税起征点（{} 元）形成的不可达区间。不含增值税收入 {} 对应企业总支出 {}，{} 对应 {}，其间的企业总支出均无对应收入。两侧最近的有效企业总支出：{}（不含增值税收入 {}）、{}（不含增值税收入 {}）",
                self.total_cost,
                self.threshold.map(|t| t.normalize().to_string()).unwrap_or_default(),
                l.net_income, l.total_cost, u.net_income, u.total_cost,
                l.total_cost, l.net_income, u.total_cost, u.net_income
            ),
            CostGapReason::Rounding => write!(
                f,
                "企业总支出 {} 无对应的不含增值税收入：增值税税额及附加税费按分舍入，不含增值税收入 {} 对应企业总支出 {}，{} 对应 {}，其间的金额不可达。两侧最近的有效企业总支出：{}（不含增值税收入 {}）、{}（不含增值税收入 {}）",
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
    #[error("金额不得为负数：{0}")]
    NegativeAmount(Decimal),
    #[error("金额 {amount} 的小数位数超过设定精度（{scale} 位）")]
    PrecisionExceeded { amount: Decimal, scale: u32 },
    #[error("目标税后实发金额 {target} 超出可测算范围（不含增值税收入上限 {max_net_income} 对应的税后实发金额为 {max_take_home}）")]
    TargetTooLarge {
        target: Decimal,
        max_net_income: Decimal,
        max_take_home: Decimal,
    },
    #[error("{0}")]
    CostUnreachable(Box<CostGap>),
    #[error("参数配置无效：{0}")]
    InvalidConfig(String),
    #[error("金额计算溢出")]
    Overflow,
}
