# 劳务报酬计算工具（labor-fee-calculator）

计算劳务报酬涉及的 **增值税、附加税、劳务报酬个税（预扣预缴）**，支持：

- **正算**：企业总成本 → 不含税开票收入、增值税、附加税、个税（收入额、减除费用、应纳税所得额、预扣率、速算扣除数、代扣个税）、实际到手；
- **反算**：实际到手 → 上述全部金额，并自动按分校准：反算出的总成本再正算，到手与目标完全一致；若因舍入/跳变不可达，取最接近的总成本并标记“不可精确达到”及差额；
- 批量表格反算（xlsx/csv）、Web 页面：规划中（见“路线图”）。

金额全部使用 `rust_decimal` 十进制运算，不使用浮点数。所有税务口径与舍入规则均可配置，默认口径及待确认事项见 [docs/ASSUMPTIONS.md](docs/ASSUMPTIONS.md)。

## 目录结构

```
crates/core   纯计算库（配置、正算、反算/校准）
crates/cli    命令行工具 labor-fee
config/default.toml   默认口径配置（同时内置于程序）
docs/ASSUMPTIONS.md   口径假设与待确认清单
```

## 构建

需要 Rust 1.85+。

```bash
cargo build --release
# 可执行文件：target/release/labor-fee
```

## 用法

```bash
# 正算：企业总成本 10100 元
labor-fee forward 10100

# 反算：希望实际到手 8397 元
labor-fee reverse 8397

# JSON 输出
labor-fee reverse 8397 --json

# 使用自定义口径
labor-fee config > my.toml        # 导出当前配置后修改
labor-fee -c my.toml forward 10100
```

示例输出（默认口径）：

```
企业总成本                10100.00
不含税开票收入            10000.00
增值税                      100.00
  附加税                         3.00  （税率 3%）
附加税合计                    3.00
—— 劳务报酬个税（预扣预缴）——
收入额                    10000.00
减除费用                   2000.00
应纳税所得额               8000.00
预扣率                         20%
速算扣除数                    0.00
代扣个税                   1600.00
实际到手金额               8397.00
```

## 配置说明

配置为 TOML，金额与比率请写成字符串（如 `"0.01"`）以保证精确。主要项：

| 段 | 关键项 | 含义 |
|---|---|---|
| `[rounding]` | `scale`、`mode`、`surcharge` | 小数位；舍入方式 `half_up`/`half_even`/`down`/`up`；附加税逐项或合计舍入 |
| `[cost]` | `basis` | 企业总成本口径：`vat_inclusive`（价税合计）或 `vat_and_surcharge_inclusive` |
| `[vat]` | `rate`、`threshold_enabled`、`threshold`、`threshold_inclusive` | 征收率与起征点 |
| `[surcharge]` | `base`、`halve`、`[[surcharge.items]]` | 计税基数（增值税额/不含税收入）、是否减半、税目列表 |
| `[iit]` | `income_base`、`deduct_surcharge`、减除费用参数、`[[iit.brackets]]` | 个税收入额基数与预扣率表 |
| `[take_home]` | `deduct_surcharge` | 到手是否扣附加税 |
| `[reverse]` | `prefer`、`search_window`、`max_cost` | 多解取值偏好、校准扫描窗口、搜索上限 |

分项附加税示例（城建 7% + 教育 3% + 地方教育 2%，减半）：

```toml
[surcharge]
base = "vat"
halve = true

[[surcharge.items]]
name = "城市维护建设税"
rate = "0.07"

[[surcharge.items]]
name = "教育费附加"
rate = "0.03"

[[surcharge.items]]
name = "地方教育附加"
rate = "0.02"
```

## 作为库使用

```rust
use labor_fee_core::{forward, Config, Reverser};
use rust_decimal_macros::dec;

let cfg = Config::default();
let b = forward(&cfg, dec!(10100))?;
let rev = Reverser::new(&cfg)?;          // 批量反算时复用
let r = rev.reverse(dec!(8397))?;
assert!(r.exact && r.breakdown.take_home == dec!(8397));
```

## 测试

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

测试包括：各分段边界（收入 800/4000、应纳税所得额 20000/50000、起征点）单元测试；proptest 性质测试（正算单调性、正算→反算往返、目标→反算→正算往返）；
0.01~200,000 元每隔 0.37 元的全量反算遍历；7 种口径组合下总成本 0~70,000 元逐分暴力表对比（验证反算取到的是最小总成本、不可达判断正确）。

## 路线图

- [x] 阶段一：core + CLI 正算/反算/校准 + 测试
- [ ] 阶段二：批量反算（xlsx/csv，取标题为“到手金额”的列，追加结果列）、CLI 子命令与 axum Web 页面（上传/下载）
