//! Spring Boot Actuator's `health`, `info`, `metrics` and `prometheus` endpoints, the `shutdown`
//! and `logfile` ones the web UI uses, and the HTTP request meter. Kotlin exposes every endpoint;
//! leaves out those describing Spring internals.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Instant;

use axum::extract::{MatchedPath, OriginalUri, Path, RawQuery, Request, State};
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{Next, from_fn_with_state};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use jiff::Timestamp;
use komga_service::{
    DISK_SPACE_THRESHOLD, Health, HealthService, Metric, MetricsService, Statistic,
};
use serde_json::{Value, json};
use tokio::fs::File;
use tokio::io::AsyncReadExt as _;
use tokio_util::sync::CancellationToken;

use crate::auth::extractors::{Admin, Authenticated};
use crate::auth::{authenticate_request, permit_anonymous};
use crate::error::ApiError;
use crate::mvc::produces::Controller;
use crate::mvc::request_params::RequestParams;
use crate::mvc::resource::{self, Resource};
use crate::routes::ApiState;

/// What the binary was built from.
#[derive(Debug, Clone, Copy)]
pub struct BuildInfo {
    pub version: &'static str,
    /// Absent when built outside a Git checkout.
    pub git: Option<GitInfo>,
}

#[derive(Debug, Clone, Copy)]
pub struct GitInfo {
    /// Absent on a detached HEAD.
    pub branch: Option<&'static str>,
    pub commit_id: &'static str,
    /// ISO 8601 with an offset, as `git show --format=%cI` prints it.
    pub commit_time: Option<&'static str>,
}

/// What the `shutdown` and `logfile` endpoints act on.
#[derive(Clone)]
pub struct ServerControl {
    pub log_file: Arc<PathBuf>,
    /// Cancelled to stop the server gracefully.
    pub shutdown: CancellationToken,
}

impl ServerControl {
    pub fn new(log_file: PathBuf) -> Self {
        Self {
            log_file: Arc::new(log_file),
            shutdown: CancellationToken::new(),
        }
    }
}

/// Spring Boot's `ActuatorMediaType` for every endpoint but Prometheus.
const PRODUCES: &[&str] = &[
    "application/vnd.spring-boot.actuator.v3+json",
    "application/vnd.spring-boot.actuator.v2+json",
    "application/json",
];

pub fn router(state: &ApiState) -> Router {
    let health = Controller::new(PRODUCES)
        .route("/actuator/health", get(health))
        .into_router()
        .route_layer(from_fn_with_state(state.auth.clone(), permit_anonymous))
        .with_state(state.health.clone());
    let info = Controller::new(PRODUCES)
        .route("/actuator/info", get(info))
        .into_router()
        .route_layer(from_fn_with_state(state.auth.clone(), authenticate_request))
        .with_state(state.build_info);
    let metrics = Controller::new(PRODUCES)
        .route("/actuator/metrics", get(metric_names))
        .route("/actuator/metrics/{name}", get(metric))
        .into_router()
        .route("/actuator/prometheus", get(prometheus))
        .route_layer(from_fn_with_state(state.auth.clone(), authenticate_request))
        .with_state(state.metrics.clone());
    let control = Controller::new(PRODUCES)
        .route("/actuator/shutdown", post(shutdown))
        .into_router()
        .merge(
            Controller::new(&["text/plain; charset=UTF-8"])
                .route("/actuator/logfile", get(log_file))
                .into_router(),
        )
        .route_layer(from_fn_with_state(state.auth.clone(), authenticate_request))
        .with_state(state.server_control.clone());
    health.merge(info).merge(metrics).merge(control)
}

/// Spring's `ShutdownEndpoint` answers before the context closes.
async fn shutdown(_admin: Admin, State(control): State<ServerControl>) -> Json<Value> {
    control.shutdown.cancel();
    Json(json!({ "message": "Shutting down, bye..." }))
}

/// Spring's `LogFileWebEndpoint`: 404 when the file does not exist.
async fn log_file(
    _admin: Admin,
    State(control): State<ServerControl>,
    OriginalUri(uri): OriginalUri,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let not_found = || ApiError::new(StatusCode::NOT_FOUND, "404 NOT_FOUND", uri.path());
    let file = File::open(control.log_file.as_path())
        .await
        .map_err(|_| not_found())?;
    let length = file
        .metadata()
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?
        .len();
    let resource = Resource::File {
        content: file.take(length),
        length,
    };
    let content_type = [(
        CONTENT_TYPE,
        HeaderValue::from_static("text/plain;charset=UTF-8"),
    )];
    resource::response(&headers, HeaderMap::from_iter(content_type), resource)
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))
}

/// Only administrators see which component is down; a `DOWN` answers 503 so that probes
/// need not parse the body.
async fn health(
    user: Option<Authenticated>,
    State(health): State<HealthService>,
) -> (StatusCode, Json<Value>) {
    let report = health.check().await;
    let up = report.is_up();
    let code = if up {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    };
    let body = if user.is_some_and(|user| user.is_admin()) {
        health_details(&report)
    } else {
        json!({ "status": status(up) })
    };
    (code, Json(body))
}

fn health_details(report: &Health) -> Value {
    let database = |up| json!({ "status": status(up), "details": { "database": "SQLite" } });
    let disk = &report.disk;
    json!({
        "status": status(report.is_up()),
        "components": {
            "db": {
                "status": status(report.database && report.tasks_database),
                "components": {
                    "main": database(report.database),
                    "tasks": database(report.tasks_database),
                },
            },
            "diskSpace": {
                "status": status(disk.is_up()),
                "details": {
                    "total": disk.total,
                    "free": disk.free,
                    "threshold": DISK_SPACE_THRESHOLD,
                    "path": disk.path,
                    "exists": disk.exists,
                },
            },
            "ping": { "status": "UP" },
        },
    })
}

fn status(up: bool) -> &'static str {
    if up { "UP" } else { "DOWN" }
}

/// Spring's simple Git mode: the abbreviated commit and its time in UTC.
async fn info(_admin: Admin, State(build): State<BuildInfo>) -> Json<Value> {
    let mut info = json!({
        "build": {
            "artifact": "komga",
            "name": "komga",
            "group": "org.gotson",
            "version": build.version,
        },
    });
    if let Some(git) = build.git {
        let time = git
            .commit_time
            .and_then(|time| time.parse::<Timestamp>().ok());
        info["git"] = json!({
            "branch": git.branch,
            "commit": {
                "id": git.commit_id.get(..7).unwrap_or(git.commit_id),
                "time": time,
            },
        });
    }
    Json(info)
}

async fn metric_names(_admin: Admin) -> Json<Value> {
    Json(json!({ "names": MetricsService::names().collect::<Vec<_>>() }))
}

/// Spring's `MetricsEndpoint`: each `tag` parameter narrows the meters to combine.
async fn metric(
    _admin: Admin,
    State(metrics): State<MetricsService>,
    Path(name): Path<String>,
    RawQuery(query): RawQuery,
    OriginalUri(uri): OriginalUri,
) -> Result<Json<Value>, ApiError> {
    let path = uri.path();
    let tags = RequestParams::parse(query.as_deref())
        .all("tag")
        .into_iter()
        .map(|tag| match tag.split_once(':') {
            Some((key, value)) => Ok((key.to_owned(), value.to_owned())),
            None => Err(ApiError::new(
                StatusCode::BAD_REQUEST,
                format!("Each tag parameter must be in the form 'key:value' but was: {tag}"),
                path,
            )),
        })
        .collect::<Result<Vec<_>, _>>()?;
    match metrics.metric(&name, &tags).await {
        Ok(Some(metric)) => Ok(Json(metric_json(&metric))),
        Ok(None) => Err(ApiError::new(StatusCode::NOT_FOUND, "Not Found", path)),
        Err(error) => Err(ApiError::internal_error(path, error)),
    }
}

fn metric_json(metric: &Metric) -> Value {
    let measurements: Vec<Value> = metric
        .measurements
        .iter()
        .map(
            |(statistic, value)| json!({ "statistic": statistic_name(*statistic), "value": value }),
        )
        .collect();
    let available_tags: Vec<Value> = metric
        .available_tags
        .iter()
        .map(|(tag, values)| json!({ "tag": tag, "values": values }))
        .collect();
    let mut body = json!({
        "name": metric.name,
        "description": metric.description,
        "measurements": measurements,
        "availableTags": available_tags,
    });
    if let Some(unit) = metric.base_unit {
        body["baseUnit"] = json!(unit);
    }
    body
}

fn statistic_name(statistic: Statistic) -> &'static str {
    match statistic {
        Statistic::Value => "VALUE",
        Statistic::Count => "COUNT",
        Statistic::TotalTime => "TOTAL_TIME",
        Statistic::Max => "MAX",
    }
}

/// The Prometheus text format, version 0.0.4.
async fn prometheus(
    _admin: Admin,
    State(metrics): State<MetricsService>,
    OriginalUri(uri): OriginalUri,
) -> Result<Response, ApiError> {
    let body = metrics
        .scrape()
        .await
        .map_err(|error| ApiError::internal_error(uri.path(), error))?;
    let content_type = "text/plain; version=0.0.4; charset=utf-8";
    Ok(([(CONTENT_TYPE, content_type)], body).into_response())
}

/// Labels only take bounded values: the route template rather than the path, and standard
/// methods.
pub(crate) async fn record_request(
    State(metrics): State<MetricsService>,
    request: Request,
    next: Next,
) -> Response {
    let method = method_label(request.method());
    // `MatchedPath` is `Arc<str>` internally, so cloning it avoids a per-request `String` allocation.
    let matched = request.extensions().get::<MatchedPath>().cloned();
    let start = Instant::now();
    let response = next.run(request).await;
    let uri = matched.as_ref().map_or("UNKNOWN", MatchedPath::as_str);
    metrics.record_request(method, uri, response.status().as_u16(), start.elapsed());
    response
}

fn method_label(method: &Method) -> &'static str {
    ["GET", "HEAD", "POST", "PUT", "PATCH", "DELETE", "OPTIONS"]
        .into_iter()
        .find(|known| *known == method.as_str())
        .unwrap_or("OTHER")
}
