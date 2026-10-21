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
    about = "劳务报酬计算：增值税、附加税、劳务报酬个税的正算与反算"
)]
struct Cli {
    /// 税务口径配置文件（TOML），缺省使用内置默认配置
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
    /// 正算（主入口）：不含税收入 → 增值税、附加税、企业总成本、个税、实际到手
    #[command(alias = "fwd")]
    Forward {
        /// 不含税收入（元，最多两位小数）
        net_income: String,
    },
    /// 辅助：按企业总成本正算（内部求不含税收入；落在空位时报错并给出原因和两侧最近的有效总成本）
    FromCost {
        /// 企业总成本（元，最多两位小数）
        total_cost: String,
    },
    /// 反算：实际到手 → 不含税收入及全部金额（多解取最小不含税收入）
    #[command(alias = "rev")]
    Reverse {
        /// 实际到手金额（元，最多两位小数）
        take_home: String,
    },
    /// 批量：读取 xlsx/xls/ods/csv 中的金额列，追加计算结果列并输出新文件（默认按“到手金额”反算）
    Batch {
        /// 输入文件
        input: PathBuf,
        /// 输出文件（缺省为 输入名_反算结果/计算结果.xlsx/csv，与输入同目录）
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// 输入列含义：take-home（到手金额，反算）| net-income（不含税收入，正算）| total-cost（企业总成本，辅助正算）
        #[arg(long, default_value = "take-home", value_parser = parse_kind)]
        input_kind: labor_fee_batch::InputKind,
        /// 读取的列标题（缺省按输入列含义：到手金额 / 不含税收入 / 企业总成本）
        #[arg(long)]
        column: Option<String>,
    },
    /// 输出当前生效的配置（TOML）
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

fn print_breakdown(b: &Breakdown) {
    println!(
        "不含税收入          {:>14}{}",
        b.net_income,
        if b.vat_exempt {
            "  （不征增值税及附加税）"
        } else {
            ""
        }
    );
    println!("增值税              {:>14}", b.vat);
    for l in &b.surcharges {
        println!(
            "  {:<16}  {:>14}  （税率 {}）",
            l.name,
            l.amount,
            pct(l.rate)
        );
    }
    println!("附加税合计          {:>14}", b.surcharge_total);
    println!("企业总成本          {:>14}", b.total_cost);
    println!("—— 劳务报酬个税（预扣预缴）——");
    println!("收入额              {:>14}", b.iit.income);
    println!("减除费用            {:>14}", b.iit.deduction);
    println!("应纳税所得额        {:>14}", b.iit.taxable_income);
    println!("预扣率              {:>14}", pct(b.iit.rate));
    println!("速算扣除数          {:>14}", b.iit.quick_deduction);
    println!("代扣个税            {:>14}", b.iit.tax);
    println!("实际到手金额        {:>14}", b.take_home);
}

fn print_reverse(r: &ReverseOutcome) {
    println!("目标到手            {:>14}", r.target);
    if r.exact {
        println!("校准结果            精确（正算到手与目标一致）");
        if let Some((a, b)) = r.exact_net_income_range {
            if a != b {
                println!("等效不含税收入区间  {a} ~ {b}（均得到相同到手，已取配置偏好值）");
            }
        }
        if !r.other_solutions.is_empty() {
            let s: Vec<String> = r.other_solutions.iter().map(|d| d.to_string()).collect();
            println!("其它可行不含税收入  {}（分段跳变导致多解）", s.join(", "));
        }
    } else {
        println!(
            "校准结果            不可精确达到，差额 {}（正算到手 − 目标）",
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
        } => {
            let name = input.to_string_lossy().to_string();
            let format = labor_fee_batch::Format::from_file_name(&name)?;
            let bytes = std::fs::read(&input).with_context(|| format!("读取文件失败：{name}"))?;
            let opts = labor_fee_batch::Options {
                kind: input_kind,
                column,
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
                    "共 {} 行：成功 {}，不可精确达到 {}，总成本不可达 {}，错误 {}，空白 {}",
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
