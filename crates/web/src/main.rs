//! 劳务报酬计算简易 Web 服务：单笔计算/测算查询 + 批量上传下载。
//!
//! API（金额均为字符串，单位元，最多两位小数）：
//! - `GET /api/forward?net_income=`：按不含增值税收入计算全部金额；
//! - `GET /api/from-cost?total_cost=`：按企业总支出测算；不可达时返回 422 及原因、两侧最近的有效总支出；
//! - `GET /api/reverse?take_home=`：按税后实发金额测算；
//! - `POST /api/batch`：multipart，字段 file、input_kind（take-home/net-income/total-cost）、column、
//!   columns（导出列及顺序，逗号分隔的列标识或列标题，缺省全部）；
//! - `GET /api/columns?input_kind=`：该输入列含义下可导出的列（标识、标题、说明，按默认顺序）。

use anyhow::Context;
use axum::{
    extract::{DefaultBodyLimit, Multipart, Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use labor_fee_core::{forward, from_total_cost, CalcError, Config, Decimal, Reverser};
use serde::Deserialize;
use std::{net::SocketAddr, path::PathBuf, str::FromStr, sync::Arc};

#[derive(Parser)]
#[command(name = "labor-fee-web", about = "劳务报酬计算 Web 页面")]
struct Args {
    /// 监听地址
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
    /// 计算参数配置文件（TOML），缺省使用内置默认参数
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// 上传文件大小上限（MB）
    #[arg(long, default_value_t = 20)]
    max_upload_mb: usize,
}

struct AppState {
    cfg: Config,
    reverser: Reverser,
}

type Shared = Arc<AppState>;

const INDEX_HTML: &str = include_str!("index.html");

fn err(status: StatusCode, msg: impl ToString) -> Response {
    (
        status,
        Json(serde_json::json!({ "error": msg.to_string() })),
    )
        .into_response()
}

fn parse_amount(s: &str) -> Result<Decimal, String> {
    let cleaned: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != ',' && *c != '，')
        .collect();
    Decimal::from_str(&cleaned).map_err(|_| format!("无法解析金额：{s}"))
}

#[derive(Deserialize)]
struct ForwardQuery {
    #[serde(alias = "amount")]
    net_income: String,
}

#[derive(Deserialize)]
struct CostQuery {
    #[serde(alias = "amount")]
    total_cost: String,
}

#[derive(Deserialize)]
struct ColumnsQuery {
    #[serde(default)]
    input_kind: Option<String>,
}

#[derive(Deserialize)]
struct ReverseQuery {
    #[serde(alias = "amount")]
    take_home: String,
}

async fn api_columns(State(st): State<Shared>, Query(q): Query<ColumnsQuery>) -> Response {
    let kind = match q
        .input_kind
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
    {
        None => labor_fee_batch::InputKind::default(),
        Some(t) => match labor_fee_batch::InputKind::parse(t) {
            Some(k) => k,
            None => return err(StatusCode::BAD_REQUEST, format!("未知输入列含义：{t}")),
        },
    };
    let cols: Vec<_> = labor_fee_batch::available_columns(&st.cfg, kind)
        .iter()
        .map(|c| serde_json::json!({"key": c.key(), "title": c.title(), "description": c.description()}))
        .collect();
    Json(serde_json::json!({"input_kind": kind.title(), "columns": cols})).into_response()
}

async fn index() -> Html<&'static str> {
    Html(INDEX_HTML)
}

async fn api_config(State(st): State<Shared>) -> impl IntoResponse {
    (
        [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
        st.cfg.to_toml_string(),
    )
}

async fn api_forward(State(st): State<Shared>, Query(q): Query<ForwardQuery>) -> Response {
    let amount = match parse_amount(&q.net_income) {
        Ok(a) => a,
        Err(m) => return err(StatusCode::BAD_REQUEST, m),
    };
    match forward(&st.cfg, amount) {
        Ok(b) => Json(b).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

/// 按企业总支出测算。空位返回 422：`{ error, reason, gap: { total_cost, reason, lower, upper, threshold } }`。
async fn api_from_cost(State(st): State<Shared>, Query(q): Query<CostQuery>) -> Response {
    let amount = match parse_amount(&q.total_cost) {
        Ok(a) => a,
        Err(m) => return err(StatusCode::BAD_REQUEST, m),
    };
    match from_total_cost(&st.cfg, amount) {
        Ok(b) => Json(b).into_response(),
        Err(CalcError::CostUnreachable(g)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(serde_json::json!({
                "error": g.to_string(),
                "reason": g.reason,
                "gap": g,
            })),
        )
            .into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

async fn api_reverse(State(st): State<Shared>, Query(q): Query<ReverseQuery>) -> Response {
    let amount = match parse_amount(&q.take_home) {
        Ok(a) => a,
        Err(m) => return err(StatusCode::BAD_REQUEST, m),
    };
    match st.reverser.reverse(amount) {
        Ok(r) => Json(r).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

/// 上传字段：file（必填）；input_kind（可选，take-home 默认 / net-income / total-cost）；
/// column（可选，缺省按 input_kind 识别“税后实发金额”/“不含增值税收入”/“企业总支出”，兼容旧列名）；
/// columns（可选，导出列及顺序，逗号分隔的列标识或列标题，缺省导出全部列）。
async fn api_batch(State(st): State<Shared>, mut mp: Multipart) -> Response {
    let mut file: Option<(String, Vec<u8>)> = None;
    let mut column: Option<String> = None;
    let mut kind = labor_fee_batch::InputKind::default();
    let mut columns_spec: Option<String> = None;
    loop {
        let field = match mp.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(e) => return err(StatusCode::BAD_REQUEST, format!("上传数据无效：{e}")),
        };
        match field.name() {
            Some("file") => {
                let name = field.file_name().unwrap_or("upload.xlsx").to_string();
                match field.bytes().await {
                    Ok(b) => file = Some((name, b.to_vec())),
                    Err(e) => {
                        return err(StatusCode::BAD_REQUEST, format!("读取上传文件失败：{e}"))
                    }
                }
            }
            Some("column") => {
                if let Ok(t) = field.text().await {
                    if !t.trim().is_empty() {
                        column = Some(t.trim().to_string());
                    }
                }
            }
            Some("columns") => {
                if let Ok(t) = field.text().await {
                    if !t.trim().is_empty() {
                        columns_spec = Some(t.trim().to_string());
                    }
                }
            }
            Some("input_kind") => {
                let t = field.text().await.unwrap_or_default();
                if !t.trim().is_empty() {
                    match labor_fee_batch::InputKind::parse(&t) {
                        Some(k) => kind = k,
                        None => {
                            return err(StatusCode::BAD_REQUEST, format!("未知输入列含义：{t}"))
                        }
                    }
                }
            }
            _ => {}
        }
    }
    let Some((name, bytes)) = file else {
        return err(StatusCode::BAD_REQUEST, "请选择要上传的文件");
    };
    let (format, out_name) = match (
        labor_fee_batch::Format::from_file_name(&name),
        labor_fee_batch::output_file_name_for(&name, kind),
    ) {
        (Ok(f), Ok(n)) => (f, n),
        (Err(e), _) | (_, Err(e)) => return err(StatusCode::BAD_REQUEST, e),
    };
    let cfg = st.cfg.clone();
    let columns = match columns_spec
        .map(|spec| labor_fee_batch::parse_columns(&spec, &cfg, kind))
        .transpose()
    {
        Ok(c) => c,
        Err(e) => return err(StatusCode::BAD_REQUEST, e),
    };
    let opts = labor_fee_batch::Options {
        kind,
        column,
        columns,
        ..Default::default()
    };
    let result =
        tokio::task::spawn_blocking(move || labor_fee_batch::process(&cfg, &bytes, format, &opts))
            .await;
    let out = match result {
        Ok(Ok(o)) => o,
        Ok(Err(e)) => return err(StatusCode::UNPROCESSABLE_ENTITY, e),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, e),
    };
    let s = &out.summary;
    let summary = format!(
        "rows={};exact={};inexact={};unreachable_cost={};errors={};empty={}",
        s.rows, s.exact, s.inexact, s.unreachable_cost, s.errors, s.empty
    );
    let disposition = format!(
        "attachment; filename=\"result.{}\"; filename*=UTF-8''{}",
        format.output_extension(),
        percent_encode(&out_name)
    );
    (
        [
            (header::CONTENT_TYPE, format.content_type().to_string()),
            (header::CONTENT_DISPOSITION, disposition),
            (header::HeaderName::from_static("x-batch-summary"), summary),
        ],
        out.bytes,
    )
        .into_response()
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

fn app(cfg: Config, max_upload_mb: usize) -> anyhow::Result<Router> {
    let reverser = Reverser::new(&cfg)?;
    let state = Arc::new(AppState { cfg, reverser });
    Ok(Router::new()
        .route("/", get(index))
        .route("/api/config", get(api_config))
        .route("/api/forward", get(api_forward))
        .route("/api/from-cost", get(api_from_cost))
        .route("/api/reverse", get(api_reverse))
        .route("/api/batch", post(api_batch))
        .route("/api/columns", get(api_columns))
        .layer(DefaultBodyLimit::max(max_upload_mb * 1024 * 1024))
        .with_state(state))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let cfg = match &args.config {
        None => Config::default(),
        Some(p) => {
            let text = std::fs::read_to_string(p)
                .with_context(|| format!("读取配置文件失败：{}", p.display()))?;
            Config::from_toml_str(&text)?
        }
    };
    let router = app(cfg, args.max_upload_mb)?;
    let listener = tokio::net::TcpListener::bind(args.listen)
        .await
        .with_context(|| format!("监听失败：{}", args.listen))?;
    println!("劳务报酬计算 Web 服务已启动：http://{}", args.listen);
    axum::serve(listener, router)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use http_body_util::BodyExt;
    use tower::ServiceExt;

    fn router() -> Router {
        app(Config::default(), 20).unwrap()
    }

    async fn body_json(resp: Response) -> serde_json::Value {
        let b = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&b).unwrap()
    }

    #[tokio::test]
    async fn forward_and_reverse_api() {
        let resp = router()
            .oneshot(
                Request::get("/api/forward?net_income=10000")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let v = body_json(resp).await;
        assert_eq!(v["take_home"], "8400.00");
        assert_eq!(v["total_cost"], "10103.00");
        let resp = router()
            .oneshot(
                Request::get("/api/reverse?take_home=8400")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let v = body_json(resp).await;
        assert_eq!(v["exact"], true);
        assert_eq!(v["breakdown"]["total_cost"], "10103.00");
        assert_eq!(v["breakdown"]["net_income"], "10000.00");
        let resp = router()
            .oneshot(
                Request::get("/api/reverse?amount=-1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn from_cost_api() {
        let get = |q: &str| {
            Request::get(format!("/api/from-cost?total_cost={q}"))
                .body(Body::empty())
                .unwrap()
        };
        let resp = router().oneshot(get("10103.00")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_json(resp).await["net_income"], "10000.00");
        let resp = router().oneshot(get("1005.00")).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNPROCESSABLE_ENTITY);
        let v = body_json(resp).await;
        assert_eq!(v["reason"], "threshold");
        assert_eq!(v["gap"]["lower"]["total_cost"], "999.99");
        assert_eq!(v["gap"]["upper"]["total_cost"], "1010.30");
        assert_eq!(v["gap"]["upper"]["net_income"], "1000.00");
        let msg = v["error"].as_str().unwrap();
        assert!(
            msg.contains("增值税起征点（1000 元）形成的不可达区间"),
            "{msg}"
        );
        assert!(
            msg.contains("不含增值税收入 999.99 对应企业总支出 999.99，1000.00 对应 1010.30"),
            "{msg}"
        );
    }

    #[tokio::test]
    async fn batch_upload_total_cost_gap_note() {
        let boundary = "XB";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"input_kind\"\r\n\r\ntotal-cost\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"c.csv\"\r\nContent-Type: text/csv\r\n\r\n企业总成本\n10103.00\n1005\n\r\n--{boundary}--\r\n"
        );
        let req = Request::post("/api/batch")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        let resp = router().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()["x-batch-summary"],
            "rows=2;exact=1;inexact=0;unreachable_cost=1;errors=0;empty=0"
        );
        let b = resp.into_body().collect().await.unwrap().to_bytes();
        let s = String::from_utf8(b.to_vec()).unwrap();
        assert!(s.contains("总支出不可达"));
        assert!(s.contains("增值税起征点（1000 元）形成的不可达区间"));
    }

    #[tokio::test]
    async fn batch_upload_csv() {
        let boundary = "XBOUNDARY";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"名单.csv\"\r\nContent-Type: text/csv\r\n\r\n到手金额\n8400\n\r\n--{boundary}--\r\n"
        );
        let req = Request::post("/api/batch")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        let resp = router().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers()["x-batch-summary"],
            "rows=1;exact=1;inexact=0;unreachable_cost=0;errors=0;empty=0"
        );
        let cd = resp.headers()[header::CONTENT_DISPOSITION]
            .to_str()
            .unwrap()
            .to_string();
        assert!(cd.contains("filename*=UTF-8''"));
        let b = resp.into_body().collect().await.unwrap().to_bytes();
        let s = String::from_utf8(b.to_vec()).unwrap();
        assert!(s.contains("10103.00"));
    }

    async fn batch_with_columns(columns: &str) -> Response {
        let boundary = "XCOLS";
        let body = format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"columns\"\r\n\r\n{columns}\r\n--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"名单.csv\"\r\nContent-Type: text/csv\r\n\r\n姓名,税后实发金额\n张三,8400\n\r\n--{boundary}--\r\n"
        );
        let req = Request::post("/api/batch")
            .header(
                header::CONTENT_TYPE,
                format!("multipart/form-data; boundary={boundary}"),
            )
            .body(Body::from(body))
            .unwrap();
        router().oneshot(req).await.unwrap()
    }

    #[tokio::test]
    async fn batch_upload_selected_columns() {
        let resp = batch_with_columns("take_home,net_income,source").await;
        assert_eq!(resp.status(), StatusCode::OK);
        let b = resp.into_body().collect().await.unwrap().to_bytes();
        let s = String::from_utf8(b.to_vec()).unwrap();
        let s = s.trim_start_matches('\u{feff}');
        let mut lines = s.lines();
        assert_eq!(
            lines.next().unwrap(),
            "税后实发金额,不含增值税收入,姓名,税后实发金额"
        );
        assert_eq!(lines.next().unwrap(), "8400.00,10000.00,张三,8400");

        let resp = batch_with_columns("不存在的列").await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn columns_api_differs_by_kind() {
        let get = |q: &'static str| async move {
            let resp = router()
                .oneshot(Request::get(q).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(resp.status(), StatusCode::OK);
            body_json(resp).await
        };
        let keys = |v: &serde_json::Value| -> Vec<String> {
            v["columns"]
                .as_array()
                .unwrap()
                .iter()
                .map(|c| c["key"].as_str().unwrap().to_string())
                .collect()
        };
        let take_home = keys(&get("/api/columns").await);
        let net = keys(&get("/api/columns?input_kind=net-income").await);
        assert_eq!(take_home.first().map(String::as_str), Some("source"));
        assert!(take_home.contains(&"diff".to_string()));
        assert!(!net.contains(&"diff".to_string()));
        let resp = router()
            .oneshot(
                Request::get("/api/columns?input_kind=x")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn index_page() {
        let resp = router()
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
