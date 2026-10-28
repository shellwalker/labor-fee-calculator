# AGENTS.md — 协作约定（开发者与 AI 代理）

本文件给在本仓库工作的人和 AI 代理看：仓库结构、构建测试命令、必须遵守的规则。用户文档见 [README.md](README.md)。

## 仓库结构

| 路径 | 内容 |
|---|---|
| `crates/core` | 纯计算库 `labor_fee_core`：配置 `Config`、按不含增值税收入计算 `forward`、按税后实发金额测算 `Reverser`、按企业总支出测算 `from_total_cost` |
| `crates/batch` | 批量测算：读 xlsx/xls/ods/csv，追加结果列，写 xlsx/csv；`columns.rs` 定义各测算方式的可导出列与 `ColumnLayout`（列选择与顺序） |
| `crates/cli` | 命令行 `labor-fee`（`forward` / `reverse` / `from-cost` / `batch` / `config`） |
| `crates/web` | axum Web 服务 `labor-fee-web`（`src/index.html` 为内嵌页面） |
| `crates/desktop` | eframe/egui 桌面程序 `labor-fee-desktop`：`state.rs` 状态逻辑、`ui/` 各页面、`batch_store.rs` 批量页缓存、`tests/ui.rs` kittest 界面测试、`tests/screenshots.rs` 截图 |
| `config/default.toml` | 默认计算参数（同时内置于程序） |
| `docs/` | `ASSUMPTIONS.md` 参数假设与政策出处、`TERMINOLOGY.md` 用语规范、`UI_DESIGN.md` 界面设计 |
| `scripts/` | 各平台构建脚本（产物到 `dist/`）、`ci-version.py` 版本号计算 |
| `.github/workflows/` | `build.yml`（可复用三平台构建）、`ci.yml`（PR 检查 / develop 预览版）、`release.yml`（main 正式版） |
| `samples/`、`examples/` | 测试表格与示例 |

## 构建与测试

需要 Rust 1.85+（桌面程序 1.95+）。

```bash
cargo build --release
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --all-targets --locked
```

- Linux 构建桌面程序需要 `pkg-config libgtk-3-dev libxkbcommon-dev`；没有 GTK3 开发包时加 `--no-default-features --features labor-fee-desktop/portal`（单独构建桌面 crate 时为 `-p labor-fee-desktop --no-default-features --features portal`）。
- macOS 目标检查：`cargo clippy --workspace --all-targets --target aarch64-apple-darwin -- -D warnings`。
- 界面截图：`LFD_SHOTS_DIR=/tmp/shots cargo test -p labor-fee-desktop --test screenshots -- --ignored`；无 GPU 时 `WGPU_BACKEND=gl`（Mesa EGL）或安装 lavapipe。
- 提交前 fmt / clippy / test 必须全部通过，CI 在三个平台上执行同样的检查。

## 必须遵守的规则

### 金额与计算

- **金额一律用 `rust_decimal::Decimal`**（测试里用 `rust_decimal_macros::dec!`），禁止 `f32` / `f64` 参与金额计算或舍入；配置文件里金额与比率写成字符串（如 `"0.01"`）。界面布局尺寸等非金额数值不受此限。
- 舍入只通过配置里的 `[rounding]` 规则进行，逐步舍入的位置与现有实现保持一致。
- 计算逻辑只写在 `crates/core`（批量逻辑在 `crates/batch`），CLI / Web / 桌面只做输入输出，不得各自实现一套计算。

### 税务参数与政策

- **所有税务参数必须可配置**（征收率、起征点、附加税费项目与费率、减征比例、城建税档位、减除费用、预扣率表等），不得在代码里写死新的税率或规则。
- **不得编造政策**：新增或修改默认参数必须有明确出处（文件号或官方问答链接），记录在 `docs/ASSUMPTIONS.md`；无法确认的标为“待确认”并告知用户，由用户决定，不要自行假设。
- 界面和输出用语遵循 `docs/TERMINOLOGY.md` 的正式税务用语（如“不含增值税收入”“应预扣预缴税额”“税后实发金额”“增值税附加税费”），新增用语先补充到该文档。

### 批量导出列

- 可导出列由 `labor_fee_batch::columns::available_columns(cfg, kind)` 统一定义；新增结果列时同时更新列标识（`key`，用于保存与 `--columns`，**已发布的标识不要改名**）、标题、说明，以及 README 中的列表。
- 列选择用 `ColumnLayout`（`order` + `hidden`）保存，必须兼容参数变化（新列自动加入、已不存在的列忽略）。

### 桌面界面

- egui 遇到字体里没有的字形会显示方框甚至 panic：界面文字只能用内嵌字体 `LaborFeeSansSC`（GB2312 + 常用符号）和 `LaborFeeIcons` 中已有的字符；新增图标需更新 `scripts/gen-assets.py` 的子集并重新生成。
- 新增界面行为需配 egui_kittest 测试（`crates/desktop/tests/ui.rs`）；查找控件优先用 `get_by_role_and_label` 避免同名标签冲突。

## 分支与发布

- `develop`：日常开发分支，所有提交进入这里（私人项目，可直接推送；若开 PR，目标也是 `develop`）。推送后 CI 发布预览版 `develop-preview`。
- `main`：只用于正式发布。**未经用户明确要求发版，不得推送 `main`，也不得开 `develop → main` 的 PR。**
- 正式版本号来自 `Cargo.toml` 的 `[workspace.package] version`；推送到 `main` 后 `release.yml` 自动创建 `v<版本>` Release。
- 不要强推、不要改写已推送的历史（撤销用新的 revert 提交）；分阶段提交，每个提交可独立通过检查。
- 不要在仓库、日志或 URL 中写入令牌等凭据；CI 只依赖 `GITHUB_TOKEN`，需要新 secret 时列出名称请用户配置，不要编造。
