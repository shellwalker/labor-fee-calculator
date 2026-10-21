# 劳务报酬计算工具（labor-fee-calculator）

计算劳务报酬涉及的 **增值税、附加税、劳务报酬个税（预扣预缴）**，支持：

- **正算**：企业总成本 → 不含税开票收入、增值税、附加税、个税（收入额、减除费用、应纳税所得额、预扣率、速算扣除数、代扣个税）、实际到手；
- **反算**：实际到手 → 上述全部金额，并自动按分校准：反算出的总成本再正算，到手与目标完全一致；若因舍入/跳变不可达，取最接近的总成本并标记“不可精确达到”及差额；
- **批量反算**：读取 xlsx/xls/ods/csv 首行标题为“到手金额”的列，在右侧追加反算结果列并输出新文件；
- **两种使用方式**：命令行 `labor-fee`、简易 Web 页面 `labor-fee-web`（单笔计算 + 上传/下载）。

金额全部使用 `rust_decimal` 十进制运算，不使用浮点数。所有税务口径与舍入规则均可配置，默认口径及待确认事项见 [docs/ASSUMPTIONS.md](docs/ASSUMPTIONS.md)。

## 目录结构

```
crates/core   纯计算库（配置、正算、反算/校准）
crates/batch  批量反算（calamine 读表格、rust_xlsxwriter 写 xlsx、csv）
crates/cli    命令行工具 labor-fee
crates/web    axum Web 服务 labor-fee-web
examples/sample.csv   批量反算示例
config/default.toml   默认口径配置（同时内置于程序）
docs/ASSUMPTIONS.md   口径假设与待确认清单
```

## 构建

需要 Rust 1.85+。

```bash
cargo build --release
# 可执行文件：target/release/labor-fee、target/release/labor-fee-web
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

# 批量反算（输出 sample_反算结果.csv；xlsx/xls/ods 输入则输出 xlsx）
labor-fee batch examples/sample.csv
labor-fee batch 名单.xlsx -o 结果.xlsx --column 到手金额
```

单笔正算示例输出（默认口径）：

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

## 批量反算说明

- 只读第一个工作表；首行为标题行，按标题精确匹配（忽略首尾空格）找到“到手金额”列（可用 `--column` 指定其它标题）。
- 金额单元格可为数值或文本，文本允许 `¥`、`元`、千分位逗号；超过两位小数、无法解析的行在“校准状态”列写“错误：…”，不中断其它行。空白行保留原样、结果列为空。
- 追加列：企业总成本、不含税开票收入、增值税、（多税目时各附加税目）、附加税合计、收入额、减除费用、应纳税所得额、预扣率、速算扣除数、代扣个税、正算到手（校验）、校准状态、差额、反算备注。
- CSV 输入自动识别 UTF-8（含 BOM）/GBK，输出 UTF-8 带 BOM（Excel 可直接打开）。表格输出为新的 xlsx，只保留原单元格的值，不保留原格式、公式与其它工作表。

## Web 页面

```bash
labor-fee-web --listen 127.0.0.1:8080 [-c my.toml] [--max-upload-mb 20]
```

浏览器打开 `http://127.0.0.1:8080`：单笔正算/反算、上传表格批量反算并自动下载结果、查看当前口径配置。接口：

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/forward?amount=10100` | 正算，返回 JSON |
| GET | `/api/reverse?amount=8397` | 反算，返回 JSON（含 `exact`、`diff`、`other_solutions`） |
| POST | `/api/batch` | multipart 字段 `file`（必填）、`column`（可选），返回结果文件；响应头 `x-batch-summary` 为统计 |
| GET | `/api/config` | 当前配置（TOML） |

服务没有登录鉴权，默认只监听本机；如需对外提供请放在带鉴权的反向代理之后。

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
0.01~200,000 元每隔 0.37 元的全量反算遍历；7 种口径组合下总成本 0~70,000 元逐分暴力表对比（验证反算取到的是最小总成本、不可达判断正确）；
批量 xlsx/csv（UTF-8/GBK/BOM）读写往返；Web 接口测试。

## 路线图

- [x] 阶段一：core + CLI 正算/反算/校准 + 测试
- [x] 阶段二：批量反算（xlsx/csv，取标题为“到手金额”的列，追加结果列）、CLI 子命令与 axum Web 页面（上传/下载）
- [ ] 后续：口径确认后固化默认配置；批量正算；Web 端在线编辑口径；部署（Docker/systemd）
