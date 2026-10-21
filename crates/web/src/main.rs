//! 劳务报酬计算简易 Web 服务：正算/反算查询 + 批量反算上传下载。

use anyhow::Context;
use axum::{
    extract::{DefaultBodyLimit, Multipart, Query, State},
    http::{header, StatusCode},
    response::{Html, IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use clap::Parser;
use labor_fee_core::{forward, Config, Decimal, Reverser};
use serde::Deserialize;
use std::{net::SocketAddr, path::PathBuf, str::FromStr, sync::Arc};

#[derive(Parser)]
#[command(name = "labor-fee-web", about = "劳务报酬计算 Web 页面")]
struct Args {
    /// 监听地址
    #[arg(long, default_value = "127.0.0.1:8080")]
    listen: SocketAddr,
    /// 税务口径配置文件（TOML），缺省使用内置默认配置
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
struct AmountQuery {
    amount: String,
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

async fn api_forward(State(st): State<Shared>, Query(q): Query<AmountQuery>) -> Response {
    let amount = match parse_amount(&q.amount) {
        Ok(a) => a,
        Err(m) => return err(StatusCode::BAD_REQUEST, m),
    };
    match forward(&st.cfg, amount) {
        Ok(b) => Json(b).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

async fn api_reverse(State(st): State<Shared>, Query(q): Query<AmountQuery>) -> Response {
    let amount = match parse_amount(&q.amount) {
        Ok(a) => a,
        Err(m) => return err(StatusCode::BAD_REQUEST, m),
    };
    match st.reverser.reverse(amount) {
        Ok(r) => Json(r).into_response(),
        Err(e) => err(StatusCode::BAD_REQUEST, e),
    }
}

/// 上传字段：file（必填），column（可选，默认“到手金额”）。
async fn api_batch(State(st): State<Shared>, mut mp: Multipart) -> Response {
    let mut file: Option<(String, Vec<u8>)> = None;
    let mut column = labor_fee_batch::DEFAULT_COLUMN.to_string();
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
                        column = t.trim().to_string();
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
        labor_fee_batch::output_file_name(&name),
    ) {
        (Ok(f), Ok(n)) => (f, n),
        (Err(e), _) | (_, Err(e)) => return err(StatusCode::BAD_REQUEST, e),
    };
    let cfg = st.cfg.clone();
    let opts = labor_fee_batch::Options { column };
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
        "rows={};exact={};inexact={};errors={};empty={}",
        s.rows, s.exact, s.inexact, s.errors, s.empty
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
        .route("/api/reverse", get(api_reverse))
        .route("/api/batch", post(api_batch))
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
                Request::get("/api/forward?amount=10103")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_json(resp).await["take_home"], "8400.00");
        let resp = router()
            .oneshot(
                Request::get("/api/reverse?amount=8400")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let v = body_json(resp).await;
        assert_eq!(v["exact"], true);
        assert_eq!(v["breakdown"]["total_cost"], "10103.00");
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
            "rows=1;exact=1;inexact=0;errors=0;empty=0"
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

    #[tokio::test]
    async fn index_page() {
        let resp = router()
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }
}
