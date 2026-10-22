# 劳务报酬计算工具（labor-fee-calculator）

计算劳务报酬涉及的 **增值税、附加税、劳务报酬个税（预扣预缴）**，支持：

- **正算（主入口）**：不含税收入 → 增值税、附加税、企业总成本、个税（收入额、减除费用、应纳税所得额、预扣率、速算扣除数、代扣个税）、实际到手；
- **反算**：实际到手 → 不含税收入 → 上述全部金额，正算回去与目标到分一致，多解取最小不含税收入；
- **按企业总成本正算（辅助）**：内部求对应的不含税收入；总成本落在空位（1000 元门槛空白区间、逐分舍入空位）时报错，说明原因并给出两侧最近的有效总成本；
- **批量**：读取 xlsx/xls/ods/csv 首行标题为“到手金额”的列（也可选“不含税收入”“企业总成本”列），在右侧追加结果列并输出新文件；
- **三种使用方式**：原生桌面程序 `labor-fee-desktop`（Windows / macOS / Linux）、命令行 `labor-fee`、简易 Web 页面 `labor-fee-web`。

金额全部使用 `rust_decimal` 十进制运算，不使用浮点数。所有税务口径与舍入规则均可配置，默认口径、个税规则出处及待确认事项见 [docs/ASSUMPTIONS.md](docs/ASSUMPTIONS.md)。

## 默认口径（用户已确认）

- 以**不含税收入 I** 为核心变量，其它金额都由它唯一确定；
- 增值税 = round(I × 1%)（I ≥ 1000 时），附加税 = round(增值税 × 3%)；
- 企业总成本 = I + 增值税 + 附加税，三项都由企业承担；
- **征收门槛**：不含税收入 ≥ 1000 元（含本数）才征收增值税和附加税，低于 1000 元两者都为 0（这是业务口径，不是法定起征点；个税不受此门槛影响）；
- 个税：居民个人劳务报酬预扣预缴（国家税务总局公告 2018 年第 61 号），每次收入 ≤ 4000 减除 800、> 4000 减除 20%，预扣率表二 20%/0、30%/2000、40%/7000；
- 实际到手 = 不含税收入 − 代扣个税。

## 目录结构

```
crates/core   纯计算库（配置、正算、反算/校准）
crates/batch  批量计算（calamine 读表格、rust_xlsxwriter 写 xlsx、csv）
crates/cli    命令行工具 labor-fee
crates/web    axum Web 服务 labor-fee-web
crates/desktop eframe/egui 桌面程序 labor-fee-desktop（内嵌中文字体）
scripts/      各平台构建脚本（产物输出到 dist/）
samples/      测试用表格（到手样例.xlsx）
examples/sample.csv   批量反算示例
config/default.toml   默认口径配置（同时内置于程序）
docs/ASSUMPTIONS.md   口径假设与待确认清单
docs/UI_DESIGN.md     桌面程序界面设计说明
```

## 构建

需要 Rust 1.85+。

```bash
cargo build --release
# 可执行文件：target/release/labor-fee、target/release/labor-fee-web
```

## 用法

```bash
# 正算（主入口）：不含税收入 10000 元
labor-fee forward 10000

# 反算：希望实际到手 8400 元
labor-fee reverse 8400

# 辅助：按企业总成本正算
labor-fee from-cost 10103
labor-fee from-cost 1005      # 落在门槛空白区间：报错并给出原因和两侧最近的有效总成本

# JSON 输出
labor-fee reverse 8400 --json

# 使用自定义口径
labor-fee config > my.toml        # 导出当前配置后修改
labor-fee -c my.toml forward 10000

# 批量反算（输出 sample_反算结果.csv；xlsx/xls/ods 输入则输出 xlsx）
labor-fee batch examples/sample.csv
labor-fee batch 名单.xlsx -o 结果.xlsx --column 到手金额
# 批量正算 / 按总成本批量正算（输出 原名_计算结果）
labor-fee batch 名单.xlsx --input-kind net-income      # 读“不含税收入”列
labor-fee batch 名单.xlsx --input-kind total-cost      # 读“企业总成本”列
```

单笔正算示例输出（默认口径）：

```
不含税收入                10000.00
增值税                      100.00
  附加税                         3.00  （税率 3%）
附加税合计                    3.00
企业总成本                10103.00
—— 劳务报酬个税（预扣预缴）——
收入额                    10000.00
减除费用                   2000.00
应纳税所得额               8000.00
预扣率                         20%
速算扣除数                    0.00
代扣个税                   1600.00
实际到手金额               8400.00
```

反算示例（默认口径）：

| 实际到手 | 不含税收入 | 增值税 | 附加税 | 企业总成本 | 代扣个税 |
|---|---|---|---|---|---|
| 959.99 | 999.99 | 0.00 | 0.00 | 999.99 | 40.00 |
| 960.00 | 1000.00 | 10.00 | 0.30 | 1010.30 | 40.00 |
| 3360 | 4000.00 | 40.00 | 1.20 | 4041.20 | 640.00 |
| 20000 | 23809.52 | 238.10 | 7.14 | 24054.76 | 3809.52 |
| 50000 | 63235.29 | 632.35 | 18.97 | 63886.61 | 13235.29 |

到手随不含税收入单调不减、每分最多变化 0.01，所以任意到手都能精确反算；门槛只影响增值税和附加税，到手在 1000 元处连续。

按企业总成本正算时，总成本 1000.00～1010.29 是 1000 元门槛形成的空白区间（不含税收入 999.99 对应 999.99，1000.00 对应 1010.30）；增值税、附加税逐分舍入还会留下单分空位（如 1061.32）。输入落在空位时报错，例如：

```
错误：企业总成本 1005.00 没有对应的不含税收入：该总成本处于 1000 元增值税征收门槛形成的空白区间——不含税收入 999.99 对应总成本 999.99，1000.00 对应 1010.30，这一段没有对应的收入。两侧最近的有效总成本：999.99（不含税收入 999.99）、1010.30（不含税收入 1000.00）
```

## 批量计算说明

- 只读第一个工作表；首行为标题行，按标题精确匹配（忽略首尾空格）。默认读“到手金额”列做反算；`--input-kind net-income`/`total-cost` 改读“不含税收入”/“企业总成本”列（`--column` 可指定其它标题）。
- 金额单元格可为数值或文本，文本允许 `¥`、`元`、千分位逗号；超过两位小数、无法解析的行“校准状态”写“错误”、“备注”写原因；企业总成本落在空位的行“校准状态”写“总成本不可达”、“备注”写原因和两侧最近的有效总成本。出错行不中断其它行。空白行保留原样、结果列为空。
- 追加列：不含税收入、增值税、（多税目时各附加税目）、附加税合计、企业总成本、收入额、减除费用、应纳税所得额、预扣率、速算扣除数、代扣个税、实际到手、校准状态、差额、备注。
- CSV 输入自动识别 UTF-8（含 BOM）/GBK，输出 UTF-8 带 BOM（Excel 可直接打开）。表格输出为新的 xlsx，只保留原单元格的值，不保留原格式、公式与其它工作表。

## Web 页面

```bash
labor-fee-web --listen 127.0.0.1:8080 [-c my.toml] [--max-upload-mb 20]
```

浏览器打开 `http://127.0.0.1:8080`：单笔正算/反算/按总成本正算、上传表格批量计算并自动下载结果、查看当前口径配置。接口：

| 方法 | 路径 | 说明 |
|---|---|---|
| GET | `/api/forward?net_income=10000` | 正算（主入口），返回 JSON |
| GET | `/api/reverse?take_home=8400` | 反算，返回 JSON（含 `exact`、`diff`、`exact_net_income_range`、`other_solutions`） |
| GET | `/api/from-cost?total_cost=10103` | 辅助：按企业总成本正算；空位返回 422，`error` 为原因说明，`gap` 含 `reason`（`threshold`/`rounding`）和两侧最近的有效总成本 `lower`/`upper` |
| POST | `/api/batch` | multipart 字段 `file`（必填）、`input_kind`（`take-home` 默认/`net-income`/`total-cost`）、`column`（可选），返回结果文件；响应头 `x-batch-summary` 为统计 |
| GET | `/api/config` | 当前配置（TOML） |

服务没有登录鉴权，默认只监听本机；如需对外提供请放在带鉴权的反向代理之后。

## 桌面程序

`labor-fee-desktop` 是单个可执行文件，计算全部复用 `crates/core` 与 `crates/batch`。

```bash
labor-fee-desktop                   # 使用“设置”中保存的口径（未保存过则为内置默认）
labor-fee-desktop --config my.toml  # 本次运行临时使用口径文件（不写入设置）
labor-fee-desktop --version         # 版本、git 短哈希、构建日期
```

界面设计说明见 [docs/UI_DESIGN.md](docs/UI_DESIGN.md)。默认浅色/深色**跟随系统**，可在设置中改为固定浅色或深色（顶栏的主题按钮也可快速切换），选择会被记住。

窗口结构：顶部品牌栏（图标、应用名、导航“正算 / 反算 / 按总成本 / 批量”、口径状态徽章、主题切换、设置），底部状态栏（口径是否为内置默认、版本与构建信息）。Windows / Linux 在窗口顶部有菜单栏；macOS 使用系统原生应用菜单。

| 页面 | 内容 |
|---|---|
| 正算 / 反算 / 按总成本 | 左：输入卡片（大号输入框、单位“元”、回车或“计算”、快捷示例金额）与最近计算记录；中：企业总成本 / 个税 / 实际到手三张指标卡、校验状态徽章、“增值税与附加税”和“个人所得税”两组明细（千分位、等宽右对齐）、算式核对；右：**试算规则**面板（按当前口径动态生成：增值税率与征收门槛、附加税 = 增值税 × 3%、企业承担、减除费用规则、预扣率表、逐步舍入、反算取最小解、真空区间，并注明国家税务总局公告 2018 年第 61 号），可收起 |
| 按总成本（无解时） | 红色警示卡：原因、数轴示意（两侧有效值与空白段）、两侧最近的有效总成本按钮（点击即填入并重算） |
| 批量 | 四步：① 拖放或选择文件 ② 选择输入列（到手金额 / 不含税收入 / 企业总成本）和列标题 ③ 执行 ④ 导出（保存到输入文件旁 / 另存为）；彩色统计卡片（总行数、成功、不可精确/不可达、错误、空白）；预览表（表头固定、斑马纹、数字右对齐、错误行红底、可横向滚动、可调列宽） |

窗口宽度 ≥ 1120 时三栏，820～1120 时两栏（试算规则移到输入下方），更窄时单列滚动；最小窗口 960×640。

### 菜单与快捷键

| 平台 | 入口 |
|---|---|
| macOS | 应用菜单：关于劳务报酬计算工具、偏好设置… `⌘,`、服务、隐藏 `⌘H`、退出 `⌘Q`；文件：打开批量文件… `⌘O`；窗口；帮助：使用说明 |
| Windows / Linux | 文件：打开批量文件… `Ctrl+O`、设置… `Ctrl+,`、退出 `Ctrl+Q`；帮助：使用说明、关于 |

拖放表格文件到窗口任意位置会直接进入批量页。

### 设置

“设置”对话框分为 增值税与附加税 / 个税（劳务报酬）/ 舍入与反算 / 外观 四组，逐项编辑全部口径：增值税征收率、征收门槛及是否含本数、附加税计税依据/减半/税目（可增删）、定额减除上限与金额、定率减除比例、预扣率表各档（可增删，最后一档无上限）、小数位、舍入方式、附加税舍入时点、反算多解取值、校准窗口与搜索上限，以及主题。

- 每项标注 **已确认 / 已核实 / 待确认 / 实现参数**；改成与内置默认不同的值后显示 **已修改**，并提供单项“默认”按钮；底部有“全部恢复默认”。
- 输入实时校验，错误项标红并在分组导航上显示错误数；有错误时不能保存。
- “保存”后立即生效（已有结果按新口径重算），“取消”放弃修改。
- 设置保存在平台标准配置目录的 `labor-fee-desktop/` 下：Linux `~/.config/`（遵循 `$XDG_CONFIG_HOME`）、macOS `~/Library/Application Support/`、Windows `%APPDATA%\`。`preferences.toml` 存主题，`rules.toml` 存口径（与内置默认相同时不保存），由程序自动维护。

测试表格：[samples/到手样例.xlsx](samples/到手样例.xlsx)（到手 500、959.99、960、3360、20000、50000 和一个非法值）。

### 图标、关于与构建信息

- 应用图标由 `scripts/gen-assets.py` 生成（`crates/desktop/assets/icon/`）：窗口图标、Windows 可执行文件图标（`build.rs` 通过 embed-resource 嵌入 `icon.ico`，MSVC 需要 rc.exe，mingw 需要 `x86_64-w64-mingw32-windres`）、macOS `AppIcon.icns`（`build-macos.sh` 打包进 .app），Linux 包内附 `labor-fee-desktop.png` 与 `.desktop` 文件。
- 窗口标题含版本号；“关于”窗口显示版本、git 短哈希与构建日期（`build.rs` 写入，可用 `LFD_GIT_HASH`、`SOURCE_DATE_EPOCH` 覆盖）、许可、字体 OFL 声明、政策依据链接和免责声明。
- 界面截图自检：`LFD_SHOTS_DIR=/tmp/shots cargo test -p labor-fee-desktop --test screenshots -- --ignored`（wgpu 离屏渲染；Linux 无 GPU 时安装 `mesa-vulkan-drivers` 用 lavapipe）。

### 中文字体与许可证

程序内嵌 `crates/desktop/assets/fonts/LaborFeeSansSC-Regular.otf`（约 3 MB）和图标字体 `LaborFeeIcons-Regular.ttf`（约 16 KB，[Noto Emoji](https://github.com/googlefonts/noto-emoji) 子集，SIL OFL 1.1，已改名 “LaborFee Icons”，只含界面用到的图标）。它是 [Noto Sans CJK SC](https://github.com/notofonts/noto-cjk) 2.004 的子集，包含 ASCII、常用标点符号、全角字符和 GB2312 全部 6763 个汉字。原字体按 **SIL Open Font License 1.1** 授权，Copyright 2014-2021 Adobe。按 OFL 要求，子集已改名为 “LaborFee Sans SC”，许可证全文见 [assets/fonts/OFL.txt](crates/desktop/assets/fonts/OFL.txt)，各平台发布包都附带一份“字体许可证-OFL.txt”。OFL 允许把字体随软件分发（包括嵌入程序），但不允许单独出售字体。

GB2312 以外的罕见字（例如个别人名用字）由 egui 的 `system_fonts` 按需从系统字体补齐。

### 构建

各平台在本机独立编译，产物都输出到 `dist/`。不需要 GitLab 流水线。桌面程序需要 Rust 1.95 及以上（eframe 0.37 的要求）。

| 平台 | 命令 | 构建依赖 | 产物 |
|---|---|---|---|
| Linux | `scripts/build-linux.sh` | `build-essential pkg-config libgtk-3-dev`（Debian/Ubuntu），或 `gcc pkgconf-pkg-config gtk3-devel`（Fedora） | `dist/linux-<arch>/labor-fee-desktop` 等，以及 `.tar.gz` |
| macOS | `scripts/build-macos.sh` | Xcode Command Line Tools（`xcode-select --install`） | `dist/macos-arm64/LaborFee.app`（仅 Apple Silicon / M 系列），以及 `labor-fee-<版本>-macos-arm64.zip` |
| Windows | `powershell -ExecutionPolicy Bypass -File scripts\build-windows.ps1` | Visual Studio 生成工具（C++ 桌面开发，MSVC） | `dist\windows-x86_64\labor-fee-desktop.exe` 等，以及 `.zip` |
| 交叉构建（可选） | `scripts/cross-build-docker.sh linux\|windows\|macos\|all` | Docker 或 Podman；加 `--no-docker` 则在本机执行，例如本机装 mingw-w64 后构建 Windows 版 | `dist/linux-x86_64/`、`dist/windows-gnu-x86_64/`、`dist/macos-arm64/` |

说明：

- **Linux**：
  - 文件对话框默认用 GTK3 后端，不依赖 xdg-desktop-portal。可加 `--features portal` 改用 portal，这样不依赖 GTK。
  - 渲染用 OpenGL（glow），虚拟机和远程桌面也能运行。
  - 运行时需要 `libgl1`（Mesa）、`libxkbcommon0`，X11 下还需要 `libxkbcommon-x11-0`，GTK3 后端另需 `libgtk-3-0`。
  - 其它参数：`--target aarch64-unknown-linux-gnu`、`--desktop-only`。
- **macOS**：
  - 只构建 Apple Silicon（M 系列，`aarch64-apple-darwin`）版本，最低 macOS 11.0；不提供 Intel / universal 版本。
  - 在 Intel Mac 上也能运行脚本，会交叉编译出 arm64 版本（自动 `rustup target add`），但产物只能在 M 系列 Mac 上运行。
  - 其它参数：`--desktop-only`。
  - 默认对 .app 做 ad-hoc 签名，`--no-sign` 可跳过。
  - 程序没有经过公证，首次打开需要右键“打开”，或在“隐私与安全性”中允许。
- **Windows**：
  - 发布构建的桌面程序是 Windows GUI 子系统程序（源码里设置了 `windows_subsystem = "windows"`），双击运行不会弹出控制台窗口。
  - 可用 `-Target aarch64-pc-windows-msvc` 或 `-DesktopOnly`。
- **交叉构建**：
  - Windows 目标为 `x86_64-pc-windows-gnu`（mingw-w64），产物只依赖系统 DLL。
  - macOS 只构建 `aarch64-apple-darwin`，使用 osxcross，需要用 `--macos-sdk <MacOSX*.sdk.tar.xz>` 自备 SDK（请自行确认 Apple SDK 的许可条款）；不提供 SDK 时跳过 macOS。装有 [rcodesign](https://github.com/indygreg/apple-platform-rs) 时自动 ad-hoc 签名，否则需在 Mac 上执行 `codesign --force --deep -s - LaborFee.app`。

## 配置说明

配置为 TOML，金额与比率请写成字符串（如 `"0.01"`）以保证精确。主要项：

| 段 | 关键项 | 含义 |
|---|---|---|
| `[rounding]` | `scale`、`mode`、`surcharge` | 小数位；舍入方式 `half_up`/`half_even`/`down`/`up`；附加税逐项或合计舍入 |
| `[vat]` | `rate`、`threshold_enabled`、`threshold`、`levy_at_threshold` | 征收率；按不含税收入判断的征收门槛金额、等于门槛时是否征收 |
| `[surcharge]` | `base`、`halve`、`[[surcharge.items]]` | 计税基数（增值税额/不含税收入）、是否减半、税目列表 |
| `[iit]` | 减除费用参数、`[[iit.brackets]]` | 减除费用与预扣率表（个税按不含税收入计算） |
| `[reverse]` | `prefer`、`search_window`、`max_net_income` | 多解取值偏好、校准扫描窗口、不含税收入搜索上限 |

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
use labor_fee_core::{forward, from_total_cost, CalcError, Config, Reverser};
use rust_decimal_macros::dec;

let cfg = Config::default();
let b = forward(&cfg, dec!(10000))?;     // 不含税收入 → 全部金额
assert_eq!(b.total_cost, dec!(10103));
assert!(matches!(from_total_cost(&cfg, dec!(1005)), Err(CalcError::CostUnreachable(_))));
let rev = Reverser::new(&cfg)?;          // 批量反算时复用
let r = rev.reverse(dec!(8400))?;
assert!(r.exact && r.breakdown.take_home == dec!(8400));
```

## 测试

```bash
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test
```

测试包括：
- 各分段边界（收入 800/1000/4000、应纳税所得额 20000/50000）单元测试；
- 不含税收入 0～200,000 元逐分正算：总成本 = 三项之和、到手 = 收入 − 个税、门槛按收入判断；
- 5 种口径下逐分验证到手单调不减、每分最多增 0.01、总成本严格递增（默认口径 0～300,000 元）；
- 到手 0.01～200,000 元每隔 0.37 元反算往返（精确、正算一致、取最小收入）；各分段起点及门槛附近 ±10 元逐分反算；
- 自定义口径下的无解标记与不相连多解；
- 按总成本正算：总成本 0～70,000 元逐分（可达则与正算一致，空位则报原因和两侧最近有效总成本；门槛空白区间正好 1030 个分）；
- proptest：收入→到手→反算、目标→反算→正算、单调性、按总成本正算；
- 批量 xlsx/csv（UTF-8/GBK/BOM）读写、按总成本批量时备注列原因；Web 接口（含 422 空位说明）；
- 桌面程序：状态逻辑（正算/反算/按总成本空位点击重算、口径文件加载与恢复、设置草稿校验/保存/取消/持久化与重新读取、试算规则随口径变化、批量同步与后台线程、预览与保存、样例表格）及 egui_kittest 界面测试（导航、输入、回车、快捷示例、规则面板收起、空位按钮、批量步骤与统计、设置对话框分组/校验/单项恢复默认/保存、菜单、关于、快捷键、最小窗口）；内嵌中文与图标字体覆盖界面与核心模块中出现的全部字符（kittest 遇到缺字会直接失败）。

## 路线图

- [x] 阶段一：core + CLI 正算/反算/校准 + 测试
- [x] 阶段二：批量反算（xlsx/csv，取标题为“到手金额”的列，追加结果列）、CLI 子命令与 axum Web 页面（上传/下载）
- [x] 以不含税收入为核心变量；按总成本正算的空位报错；批量正算/按总成本批量
- [x] 原生桌面程序（eframe/egui，Windows / macOS / Linux）与各平台构建脚本
- [ ] 后续：Web 端在线编辑口径；部署（Docker/systemd）；Windows/macOS 代码签名
