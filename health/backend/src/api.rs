//! HTTP API: JSON endpoints + static SPA serving.

use std::collections::HashMap;
use std::path::PathBuf;

use axum::extract::{Query, State};
use axum::response::Json;
use axum::routing::get;
use axum::Router;
use serde::{Deserialize, Serialize};
use tower_http::services::{ServeDir, ServeFile};

use crate::metrics::{self, Sample, Sampler};

#[derive(Clone)]
pub struct AppState {
    pub sampler: std::sync::Arc<Sampler>,
    pub static_dir: PathBuf,
    pub hostname: String,
    pub kernel: String,
}

pub fn router(state: AppState) -> Router {
    let index = state.static_dir.join("index.html");
    let files = ServeDir::new(&state.static_dir).fallback(ServeFile::new(index));
    Router::new()
        .route("/api/data", get(api_data))
        .route("/api/status", get(api_status))
        .route("/api/health", get(|| async { "ok" }))
        .fallback_service(files)
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

fn unix_now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// /api/data
// ---------------------------------------------------------------------------

#[derive(Deserialize)]
pub struct DataQuery {
    /// unix seconds (float ok). Defaults to now - 15 min.
    from: Option<f64>,
    /// unix seconds. Defaults to now.
    to: Option<f64>,
}

#[derive(Serialize)]
pub struct SeriesData {
    pub from: f64,
    pub to: f64,
    /// seconds per bucket
    pub step: f64,
    /// bucket midpoint timestamps
    pub t: Vec<f64>,
    /// series name -> values aligned with `t` (null where missing)
    pub series: HashMap<String, Vec<Option<f64>>>,
}

#[derive(Serialize)]
pub struct ApiData {
    pub status: Status,
    pub series: SeriesData,
}

async fn api_data(State(st): State<AppState>, Query(q): Query<DataQuery>) -> Json<ApiData> {
    let now = unix_now();
    let to = q.to.unwrap_or(now).min(now);
    let from = q.from.unwrap_or(to - 900.0).min(to);
    let buckets = st.sampler.query(from, to, 1500).await;
    let series = build_series(&buckets, from, to);
    let status = status_from(st.sampler.last().await, &st).await;
    Json(ApiData { status, series })
}

fn build_series(buckets: &[Option<Sample>], from: f64, to: f64) -> SeriesData {
    let n = buckets.len();
    let step = if n > 0 { (to - from) / n as f64 } else { 1.0 };
    let mut names: Vec<String> = Vec::new();
    let mut values: HashMap<String, Vec<Option<f64>>> = HashMap::new();
    let mut t = Vec::with_capacity(n);
    for (i, b) in buckets.iter().enumerate() {
        t.push(from + (i as f64 + 0.5) * step);
        if let Some(s) = b {
            for (name, val) in metrics::series_of(s) {
                if !names.contains(&name) {
                    names.push(name.clone());
                    values.insert(name.clone(), vec![None; n]);
                }
                if let Some(v) = values.get_mut(&name) {
                    v[i] = Some(val);
                }
            }
        }
    }
    let len = n;
    let series = names
        .into_iter()
        .map(|name| {
            let v = values.remove(&name).unwrap_or_else(|| vec![None; len]);
            (name, v)
        })
        .collect();
    SeriesData {
        from,
        to,
        step,
        t,
        series,
    }
}

// ---------------------------------------------------------------------------
// /api/status
// ---------------------------------------------------------------------------

#[derive(Serialize)]
pub struct Status {
    pub hostname: String,
    pub kernel: String,
    pub now: f64,
    pub uptime: f64,
    pub load: Vec<f32>,
    pub cpu: f32,
    pub mem: MemStatus,
    pub net: Vec<NetStatus>,
    pub disk: Vec<DiskStatus>,
    pub fs: Vec<FsStatus>,
    pub temps: Vec<TempStatus>,
    pub sample_interval: f64,
}

#[derive(Serialize, Default)]
pub struct MemStatus {
    pub total: u64,
    pub used: u64,
    pub available: u64,
    pub cache: u64,
    pub buffers: u64,
    pub free: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

#[derive(Serialize)]
pub struct NetStatus {
    pub name: String,
    pub rx_bps: f64,
    pub tx_bps: f64,
}

#[derive(Serialize)]
pub struct DiskStatus {
    pub name: String,
    pub read_bps: f64,
    pub write_bps: f64,
    pub read_iops: f64,
    pub write_iops: f64,
    pub util_pct: f32,
}

#[derive(Serialize)]
pub struct FsStatus {
    pub device: String,
    pub mount: String,
    pub fstype: String,
    pub total: u64,
    pub used: u64,
    pub avail: u64,
}

#[derive(Serialize)]
pub struct TempStatus {
    pub name: String,
    pub celsius: f32,
}

async fn api_status(State(st): State<AppState>) -> Json<Status> {
    let last = st.sampler.last().await;
    Json(status_from(last, &st).await)
}

async fn status_from(last: Option<Sample>, st: &AppState) -> Status {
    let interval = st.sampler.interval_secs();
    let Some(s) = last else {
        return Status {
            hostname: st.hostname.clone(),
            kernel: st.kernel.clone(),
            now: unix_now(),
            uptime: 0.0,
            load: vec![0.0, 0.0, 0.0],
            cpu: 0.0,
            mem: MemStatus::default(),
            net: vec![],
            disk: vec![],
            fs: vec![],
            temps: vec![],
            sample_interval: interval,
        };
    };
    Status {
        hostname: st.hostname.clone(),
        kernel: st.kernel.clone(),
        now: s.t,
        uptime: s.uptime,
        load: s.load.to_vec(),
        cpu: s.cpu_total,
        mem: MemStatus {
            total: s.mem.total,
            used: s.mem.used(),
            available: s.mem.available,
            cache: s.mem.cache,
            buffers: s.mem.buffers,
            free: s.mem.free,
            swap_total: s.mem.swap_total,
            swap_used: s.mem.swap_used,
        },
        net: s.net
            .iter()
            .map(|n| NetStatus {
                name: n.name.clone(),
                rx_bps: n.rx_bps,
                tx_bps: n.tx_bps,
            })
            .collect(),
        disk: s
            .disk
            .iter()
            .map(|d| DiskStatus {
                name: d.name.clone(),
                read_bps: d.read_bps,
                write_bps: d.write_bps,
                read_iops: d.read_iops,
                write_iops: d.write_iops,
                util_pct: d.util_pct,
            })
            .collect(),
        fs: s
            .fs
            .iter()
            .map(|f| FsStatus {
                device: f.device.clone(),
                mount: f.mount.clone(),
                fstype: f.fstype.clone(),
                total: f.total,
                used: f.used,
                avail: f.avail,
            })
            .collect(),
        temps: s
            .temps
            .iter()
            .map(|t| TempStatus {
                name: t.name.clone(),
                celsius: t.celsius,
            })
            .collect(),
        sample_interval: interval,
    }
}


