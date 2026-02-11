use actix_web::{HttpResponse};
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

static REQUEST_COUNT: AtomicU64 = AtomicU64::new(0);
static ERROR_COUNT: AtomicU64 = AtomicU64::new(0);

pub fn increment_request_count() {
    REQUEST_COUNT.fetch_add(1, Ordering::Relaxed);
}

pub fn increment_error_count() {
    ERROR_COUNT.fetch_add(1, Ordering::Relaxed);
}

#[derive(Serialize, Deserialize, utoipa::ToSchema)]
pub struct MetricsResponse {
    pub http_requests_total: u64,
    pub http_errors_total: u64,
}

#[utoipa::path(
    get,
    path = "/metrics",
    tag = "Health",
    responses(
        (status = 200, description = "System metrics", body = MetricsResponse)
    )
)]
pub async fn get_metrics() -> HttpResponse {
    let requests = REQUEST_COUNT.load(Ordering::Relaxed);
    let errors = ERROR_COUNT.load(Ordering::Relaxed);
    
    let metrics = format!(
        "# HELP http_requests_total Total number of HTTP requests\n\
         # TYPE http_requests_total counter\n\
         http_requests_total {}\n\
         \n\
         # HELP http_errors_total Total number of HTTP errors\n\
         # TYPE http_errors_total counter\n\
         http_errors_total {}\n",
        requests, errors
    );
    
    HttpResponse::Ok()
        .content_type("text/plain; version=0.0.4")
        .body(metrics)
}
