//! API client types + fetch. Mirrors `health-backend/src/api.rs`.

use std::collections::HashMap;

use serde::Deserialize;

#[derive(Clone, Debug, Default)]
pub struct ApiData {
    pub status: Option<Status>,
    pub series: Option<Series>,
    pub error: Option<String>,
}

// Wire-format types: several fields are part of the API contract but not
// (yet) rendered; keep them for completeness.
#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct Status {
    pub hostname: String,
    pub kernel: String,
    pub now: f64,
    pub uptime: f64,
    pub load: Vec<f64>,
    pub cpu: f64,
    pub mem: MemStatus,
    pub net: Vec<NetStatus>,
    pub disk: Vec<DiskStatus>,
    pub fs: Vec<FsStatus>,
    pub temps: Vec<TempStatus>,
    pub sample_interval: f64,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default, Deserialize)]
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

#[derive(Clone, Debug, Deserialize)]
pub struct NetStatus {
    pub name: String,
    pub rx_bps: f64,
    pub tx_bps: f64,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct DiskStatus {
    pub name: String,
    pub read_bps: f64,
    pub write_bps: f64,
    pub read_iops: f64,
    pub write_iops: f64,
    pub util_pct: f32,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Deserialize)]
pub struct FsStatus {
    pub device: String,
    pub mount: String,
    pub fstype: String,
    pub total: u64,
    pub used: u64,
    pub avail: u64,
}

#[derive(Clone, Debug, Deserialize)]
pub struct TempStatus {
    pub name: String,
    pub celsius: f32,
}

#[allow(dead_code)]
#[derive(Clone, Debug, Default, Deserialize)]
pub struct Series {
    pub from: f64,
    pub to: f64,
    pub step: f64,
    pub t: Vec<f64>,
    /// series name -> values aligned with `t` (None where missing)
    pub series: HashMap<String, Vec<Option<f64>>>,
}

/// Fetch `/api/data` covering the last `range_secs`.
pub async fn fetch(range_secs: f64) -> Result<ApiData, String> {
    let now = js_sys::Date::now() / 1000.0;
    let from = now - range_secs;
    let url = format!("/api/data?from={from:.0}&to={now:.0}");
    let resp = gloo_net::http::Request::get(&url)
        .send()
        .await
        .map_err(|e| format!("request failed: {e}"))?;
    let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;
    let parsed: RawApiData =
        serde_json::from_str(&text).map_err(|e| format!("bad json: {e}"))?;
    Ok(ApiData {
        status: parsed.status,
        series: parsed.series,
        error: None,
    })
}

#[derive(Deserialize)]
struct RawApiData {
    status: Option<Status>,
    series: Option<Series>,
}

/// Last non-null value of a series, or None.
pub fn series_last(data: &Option<ApiData>, name: &str) -> Option<f64> {
    data.as_ref()?
        .series
        .as_ref()?
        .series
        .get(name)?
        .iter()
        .rev()
        .find_map(|v| *v)
}

/// Mirror of the backend's `sanitize`: series names must match exactly.
pub fn sanitize(s: &str) -> String {
    let out: String = s
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect();
    out.trim_matches('-').to_string()
}

pub fn mount_series(mount: &str) -> String {
    let s = sanitize(mount);
    if s.is_empty() {
        "root".to_string()
    } else {
        s
    }
}
