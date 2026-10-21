use rust_decimal::Decimal;
use thiserror::Error;

/// 计算与配置错误。
#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum CalcError {
    #[error("金额不能为负数：{0}")]
    NegativeAmount(Decimal),
    #[error("金额 {amount} 的小数位超过配置的精度（{scale} 位）")]
    PrecisionExceeded { amount: Decimal, scale: u32 },
    #[error("目标到手金额 {target} 超出可计算范围（企业总成本上限 {max_cost} 时到手仅 {max_take_home}）")]
    TargetTooLarge {
        target: Decimal,
        max_cost: Decimal,
        max_take_home: Decimal,
    },
    #[error("配置无效：{0}")]
    InvalidConfig(String),
    #[error("金额计算溢出")]
    Overflow,
}
