//! 劳务报酬计算命令行工具。

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use labor_fee_core::{
    forward, from_total_cost, Breakdown, CalcError, Config, Decimal, ReverseOutcome, Reverser,
};
use std::path::PathBuf;
use std::str::FromStr;

#[derive(Parser)]
#[command(
    name = "labor-fee",
    version,
    about = "劳务报酬计算：增值税及附加税费、个人所得税预扣预缴的计算与测算"
)]
struct Cli {
    /// 计算参数配置文件（TOML），缺省使用内置默认参数
    #[arg(short, long, global = true)]
    config: Option<PathBuf>,
    /// 以 JSON 输出
    #[arg(long, global = true)]
    json: bool,
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// 按不含增值税收入计算：增值税税额、附加税费、企业总支出、应预扣预缴税额、税后实发金额
    #[command(alias = "fwd")]
    Forward {
        /// 不含增值税收入（元，最多两位小数）
        net_income: String,
    },
    /// 按企业总支出测算不含增值税收入（金额不可达时报错，并给出原因和两侧最近的有效总支出）
    FromCost {
        /// 企业总支出（元，最多两位小数）
        total_cost: String,
    },
    /// 按税后实发金额测算不含增值税收入及全部金额（多解时取最小不含增值税收入）
    #[command(alias = "rev")]
    Reverse {
        /// 税后实发金额（元，最多两位小数）
        take_home: String,
    },
    /// 批量测算：读取 xlsx/xls/ods/csv 中的金额列，追加结果列并输出新文件（默认读取“税后实发金额”列）
    Batch {
        /// 输入文件（使用 --list-columns 时可省略）
        #[arg(required_unless_present = "list_columns")]
        input: Option<PathBuf>,
        /// 输出文件（缺省为 输入名_测算结果/计算结果.xlsx/csv，与输入同目录）
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// 输入列含义：take-home（税后实发金额）| net-income（不含增值税收入）| total-cost（企业总支出）
        #[arg(long, default_value = "take-home", value_parser = parse_kind)]
        input_kind: labor_fee_batch::InputKind,
        /// 读取的列标题（缺省按输入列含义识别：税后实发金额 / 不含增值税收入 / 企业总支出，兼容旧列名）
        #[arg(long)]
        column: Option<String>,
        /// 导出列及顺序：逗号分隔的列标识或列标题，如 `source,net_income,iit_tax,take_home`
        /// 或 `原表各列,不含增值税收入,应预扣预缴税额`（缺省导出全部列；可用的列见 --list-columns）
        #[arg(long)]
        columns: Option<String>,
        /// 列出该输入列含义下可导出的列（标识与标题，按默认顺序）后退出
        #[arg(long)]
        list_columns: bool,
    },
    /// 输出当前生效的计算参数（TOML）
    Config,
}

fn parse_kind(s: &str) -> std::result::Result<labor_fee_batch::InputKind, String> {
    labor_fee_batch::InputKind::parse(s)
        .ok_or_else(|| format!("未知输入列含义：{s}（可选 take-home / net-income / total-cost）"))
}

fn parse_amount(s: &str) -> Result<Decimal> {
    let cleaned: String = s.chars().filter(|c| *c != ',' && *c != '，').collect();
    Decimal::from_str(cleaned.trim()).with_context(|| format!("无法解析金额：{s}"))
}

fn load_config(path: Option<&PathBuf>) -> Result<Config> {
    match path {
        None => Ok(Config::default()),
        Some(p) => {
            let text = std::fs::read_to_string(p)
                .with_context(|| format!("读取配置文件失败：{}", p.display()))?;
            Ok(Config::from_toml_str(&text)?)
        }
    }
}

fn pct(d: Decimal) -> String {
    format!("{}%", (d * Decimal::ONE_HUNDRED).normalize())
}

/// 按显示宽度（中文字符计 2）左对齐补齐标签。
fn label(s: &str) -> String {
    const WIDTH: usize = 24;
    let w: usize = s.chars().map(|c| if c.is_ascii() { 1 } else { 2 }).sum();
    format!("{s}{}", " ".repeat(WIDTH.saturating_sub(w)))
}

fn line(name: &str, value: impl std::fmt::Display) {
    println!("{}{:>14}", label(name), value);
}

fn print_breakdown(b: &Breakdown) {
    println!("—— 增值税及附加税费（企业承担）——");
    line("不含增值税收入", b.net_income);
    line("增值税税额", b.vat);
    if b.vat_exempt {
        println!("  （未达到增值税起征点，不征收增值税及附加税费）");
    }
    for l in &b.surcharges {
        if l.relief_rate.is_zero() {
            println!(
                "{}{:>14}  （费率 {}）",
                label(&format!("  {}", l.name)),
                l.amount,
                pct(l.statutory_rate)
            );
        } else {
            println!(
                "{}{:>14}  （法定费率 {}，减征 {}，实际征收率 {}；减征前 {}，减征额 {}）",
                label(&format!("  {}", l.name)),
                l.amount,
                pct(l.statutory_rate),
                pct(l.relief_rate),
                pct(l.rate),
                l.amount_before_relief,
                l.relief_amount
            );
        }
    }
    if !b.surcharge_relief.is_zero() {
        line("附加税费合计（减征前）", b.surcharge_before_relief);
        line("附加税费减征额", b.surcharge_relief);
    }
    line("增值税附加税费合计", b.surcharge_total);
    line("企业总支出", b.total_cost);
    println!("—— 个人所得税（居民个人劳务报酬所得预扣预缴）——");
    line("每次收入", b.iit.income);
    line("减除费用", b.iit.deduction);
    line("预扣预缴应纳税所得额", b.iit.taxable_income);
    line("预扣率", pct(b.iit.rate));
    line("速算扣除数", b.iit.quick_deduction);
    line("应预扣预缴税额", b.iit.tax);
    line("税后实发金额", b.take_home);
}

fn print_reverse(r: &ReverseOutcome) {
    line("目标税后实发金额", r.target);
    if r.exact {
        println!("{}精确匹配（计算结果与目标一致）", label("测算结果"));
        if let Some((a, b)) = r.exact_net_income_range {
            if a != b {
                println!(
                    "{}{a} ~ {b}（税后实发金额相同，已按参数取值）",
                    label("等效不含增值税收入区间")
                );
            }
        }
        if !r.other_solutions.is_empty() {
            let s: Vec<String> = r.other_solutions.iter().map(|d| d.to_string()).collect();
            println!(
                "{}{}（分段变化导致多解）",
                label("其他可行不含增值税收入"),
                s.join("、")
            );
        }
    } else {
        println!(
            "{}无法精确匹配，差额 {}（计算结果 − 目标）",
            label("测算结果"),
            r.diff
        );
    }
    println!();
    print_breakdown(&r.breakdown);
}

fn run() -> Result<()> {
    let cli = Cli::parse();
    let cfg = load_config(cli.config.as_ref())?;
    match cli.cmd {
        Cmd::Config => print!("{}", cfg.to_toml_string()),
        Cmd::Forward { net_income } => {
            let b = forward(&cfg, parse_amount(&net_income)?)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&b)?);
            } else {
                print_breakdown(&b);
            }
        }
        Cmd::FromCost { total_cost } => match from_total_cost(&cfg, parse_amount(&total_cost)?) {
            Ok(b) => {
                if cli.json {
                    println!("{}", serde_json::to_string_pretty(&b)?);
                } else {
                    print_breakdown(&b);
                }
            }
            Err(CalcError::CostUnreachable(g)) => {
                if cli.json {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&serde_json::json!({
                            "error": "cost_unreachable",
                            "message": g.to_string(),
                            "gap": g,
                        }))?
                    );
                    std::process::exit(2);
                }
                anyhow::bail!("{g}");
            }
            Err(e) => return Err(e.into()),
        },
        Cmd::Batch {
            input,
            output,
            input_kind,
            column,
            columns,
            list_columns,
        } => {
            if list_columns {
                let avail = labor_fee_batch::available_columns(&cfg, input_kind);
                if cli.json {
                    let v: Vec<_> = avail
                        .iter()
                        .map(|c| serde_json::json!({"key": c.key(), "title": c.title()}))
                        .collect();
                    println!("{}", serde_json::to_string_pretty(&v)?);
                } else {
                    println!("{}可导出的列（默认全部、按此顺序）：", input_kind.title());
                    for c in &avail {
                        println!("  {:<18} {}", c.key(), c.title());
                    }
                }
                return Ok(());
            }
            let input = input.context("请指定输入文件")?;
            let columns = columns
                .map(|spec| labor_fee_batch::parse_columns(&spec, &cfg, input_kind))
                .transpose()
                .map_err(anyhow::Error::msg)?;
            let name = input.to_string_lossy().to_string();
            let format = labor_fee_batch::Format::from_file_name(&name)?;
            let bytes = std::fs::read(&input).with_context(|| format!("读取文件失败：{name}"))?;
            let opts = labor_fee_batch::Options {
                kind: input_kind,
                column,
                columns,
                ..Default::default()
            };
            let out = labor_fee_batch::process(&cfg, &bytes, format, &opts)?;
            let out_path = match output {
                Some(p) => p,
                None => {
                    input.with_file_name(labor_fee_batch::output_file_name_for(&name, input_kind)?)
                }
            };
            std::fs::write(&out_path, &out.bytes)
                .with_context(|| format!("写出文件失败：{}", out_path.display()))?;
            let s = &out.summary;
            if cli.json {
                println!(
                    "{}",
                    serde_json::json!({
                        "output": out_path.display().to_string(),
                        "rows": s.rows, "exact": s.exact, "inexact": s.inexact,
                        "unreachable_cost": s.unreachable_cost,
                        "errors": s.errors, "empty": s.empty,
                    })
                );
            } else {
                println!("已输出：{}", out_path.display());
                println!(
                    "共 {} 行：成功 {}，无法精确匹配 {}，总支出不可达 {}，数据错误 {}，空白 {}",
                    s.rows, s.exact, s.inexact, s.unreachable_cost, s.errors, s.empty
                );
            }
        }
        Cmd::Reverse { take_home } => {
            let r = Reverser::new(&cfg)?.reverse(parse_amount(&take_home)?)?;
            if cli.json {
                println!("{}", serde_json::to_string_pretty(&r)?);
            } else {
                print_reverse(&r);
            }
        }
    }
    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("错误：{e:#}");
        std::process::exit(1);
    }
}
