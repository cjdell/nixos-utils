//! Health dashboard — dioxus SPA.
//!
//! Polls `/api/data` every second and renders a grid of canvas charts
//! (CPU, memory, network, disk IO, disk space, load, temperature) with a
//! selectable time range.

mod api;
mod chart;
mod theme;

use api::{ApiData, FsStatus, NetStatus};
use chart::{Chart, ChartOpts, Serie, YFmt};
use dioxus::prelude::*;
use theme::*;

const RANGES: [(f64, &str); 6] = [
    (60.0, "1m"),
    (300.0, "5m"),
    (900.0, "15m"),
    (3600.0, "1h"),
    (21600.0, "6h"),
    (86400.0, "24h"),
];

fn main() {
    dioxus::launch(App);
}

fn App() -> Element {
    let data = use_signal(|| None::<ApiData>);
    use_context_provider(move || data.clone());
    let range = use_signal(|| 900.0f64);

    // One polling loop for the app's lifetime; the range signal is read fresh
    // on every iteration so switching ranges takes effect on the next tick.
    let _ = use_effect(move || {
        let mut data = data.clone();
        let range = range.clone();
        spawn(async move {
            loop {
                let r = *range.read();
                match api::fetch(r).await {
                    Ok(d) => {
                        if let Some(st) = d.status.as_ref() {
                            web_sys::window()
                                .and_then(|w| w.document())
                                .map(|doc| doc.set_title(&format!("Health · {}", st.hostname)));
                        }
                        data.set(Some(d));
                    }
                    Err(e) => {
                        let mut w = data.write();
                        match w.as_mut() {
                            Some(dd) => dd.error = Some(e),
                            None => *w = Some(ApiData { error: Some(e), ..Default::default() }),
                        }
                    }
                }
                gloo_timers::future::TimeoutFuture::new(1000).await;
            }
        });
    });

    rsx! {
        div { class: "app",
            Header { range }
            Dashboard { }
            Footer { }
        }
    }
}

// ---------------------------------------------------------------------------
// Header
// ---------------------------------------------------------------------------

#[component]
fn StatPill(label: String, value: String) -> Element {
    rsx! {
        div { class: "pill",
            span { class: "pill-label", "{label}" }
            span { class: "pill-value", "{value}" }
        }
    }
}

#[component]
fn Header(range: Signal<f64>) -> Element {
    let data = use_context::<Signal<Option<ApiData>>>();
    let d = data.read();
    let st = d.as_ref().and_then(|d| d.status.as_ref());

    let hostname = st.map(|s| s.hostname.clone()).unwrap_or_else(|| "…".into());
    let kernel = st.map(|s| s.kernel.clone()).unwrap_or_default();
    let uptime = st.map(|s| theme::fmt_uptime(s.uptime)).unwrap_or_default();
    let cpu = st.map(|s| s.cpu).unwrap_or(0.0);
    let load = st
        .map(|s| s.load.iter().map(|v| format!("{v:.2}")).collect::<Vec<_>>().join(" "))
        .unwrap_or_default();
    let mem = st
        .map(|s| format!("{} / {}", theme::fmt_bytes(s.mem.used as f64), theme::fmt_bytes(s.mem.total as f64)))
        .unwrap_or_default();
    let clock = st.map(|s| theme::fmt_time(s.now, true)).unwrap_or_default();
    let err = d.as_ref().and_then(|d| d.error.clone());
    let buttons: Vec<(f64, &'static str, bool, Signal<f64>)> = RANGES
        .iter()
        .map(|(s, l)| (*s, *l, (range() - *s).abs() < 0.5, range.clone()))
        .collect();

    rsx! {
        header { class: "header",
            div { class: "header-left",
                div { class: "host",
                    div { class: "host-name", "{hostname}" }
                    div { class: "host-sub", "kernel {kernel} · up {uptime}" }
                }
                div { class: "pills",
                    StatPill { label: "cpu".to_string(), value: fmt_pct(cpu) }
                    StatPill { label: "load".to_string(), value: load }
                    StatPill { label: "mem".to_string(), value: mem }
                }
            }
            div { class: "header-right",
                div { class: "ranges",
                    for (secs, label, active, r) in buttons {
                        button {
                            class: if active { "range-btn active" } else { "range-btn" },
                            onclick: move |_| { let mut r = r; r.set(secs); },
                            "{label}"
                        }
                    }
                }
                div { class: "clock", "{clock}" }
            }
        }
        if let Some(e) = err {
            div { class: "error-banner", "⚠ {e}" }
        }
    }
}

// ---------------------------------------------------------------------------
// Dashboard
// ---------------------------------------------------------------------------

#[component]
fn Dashboard() -> Element {
    let data = use_context::<Signal<Option<ApiData>>>();
    let d = data.read();
    let Some(api_data) = d.as_ref() else {
        return rsx! { div { class: "loading", "collecting…" } };
    };
    let Some(status) = api_data.status.as_ref() else {
        return rsx! { div { class: "loading", "waiting for status…" } };
    };
    let Some(sd) = api_data.series.as_ref() else {
        return rsx! { div { class: "loading", "waiting for series…" } };
    };
    let _ = sd;

    let ncores = d
        .as_ref()
        .and_then(|dd| dd.series.as_ref())
        .map(|s| s.series.keys().filter(|k| k.starts_with("cpu.core.")).count())
        .unwrap_or(0);

    // network cards
    let main_ifaces: Vec<&NetStatus> = status
        .net
        .iter()
        .filter(|n| !is_virtual(&n.name) && iface_active(api_data, &n.name))
        .collect();
    let virt_ifaces: Vec<&NetStatus> = status
        .net
        .iter()
        .filter(|n| is_virtual(&n.name) && iface_active(api_data, &n.name))
        .collect();

    // swap card
    let show_swap = status.mem.swap_total > 0;

    // fs cards
    let fs_cards = status
        .fs
        .iter()
        .map(|f| fs_card(f, &*d))
        .collect::<Vec<_>>();

    rsx! {
        div { class: "grid",
            // ---- CPU -------------------------------------------------------
            Chart {
                id: "cpu".to_string(),
                title: "CPU".to_string(),
                cur: format!("{} · {} cores", fmt_pct(status.cpu), if ncores > 0 { ncores } else { 1 }),
                series: cpu_series(ncores),
                opts: ChartOpts {
                    stack: ncores > 0,
                    yfmt: YFmt::Percent,
                    ymin: Some(0.0),
                    ymax: Some(if ncores > 0 { ncores as f64 * 100.0 } else { 100.0 }),
                },
                height: 180,
            }

            // ---- Memory ----------------------------------------------------
            Chart {
                id: "mem".to_string(),
                title: "Memory".to_string(),
                cur: format!(
                    "{} / {}",
                    theme::fmt_bytes(status.mem.used as f64),
                    theme::fmt_bytes(status.mem.total as f64)
                ),
                series: vec![
                    Serie { name: "mem.used".into(), label: "used".into(), color: color(1), dash: false, fill: true },
                    Serie { name: "mem.cache".into(), label: "cache".into(), color: color(2), dash: false, fill: true },
                    Serie { name: "mem.buffers".into(), label: "buffers".into(), color: color(5), dash: false, fill: true },
                    Serie { name: "mem.free".into(), label: "free".into(), color: color(4), dash: false, fill: true },
                ],
                opts: ChartOpts { stack: true, yfmt: YFmt::Bytes, ymin: Some(0.0), ymax: Some(status.mem.total as f64) },
                height: 180,
            }

            // ---- Swap -------------------------------------------------------
            if show_swap {
                Chart {
                    id: "swap".to_string(),
                    title: "Swap".to_string(),
                    cur: format!(
                        "{} / {}",
                        theme::fmt_bytes(status.mem.swap_used as f64),
                        theme::fmt_bytes(status.mem.swap_total as f64)
                    ),
                    series: vec![
                        Serie { name: "mem.swap".into(), label: "used".into(), color: color(6), dash: false, fill: true },
                    ],
                    opts: ChartOpts { stack: false, yfmt: YFmt::Bytes, ymin: Some(0.0), ymax: Some(status.mem.swap_total as f64) },
                    height: 140,
                }
            }

            // ---- Load --------------------------------------------------------
            Chart {
                id: "load".to_string(),
                title: "Load average".to_string(),
                cur: format!("1m {}", status.load.first().copied().map(|v| format!("{v:.2}")).unwrap_or_default()),
                series: vec![
                    Serie { name: "load.1".into(), label: "1m".into(), color: color(0), dash: false, fill: true },
                    Serie { name: "load.5".into(), label: "5m".into(), color: color(1), dash: true, fill: false },
                    Serie { name: "load.15".into(), label: "15m".into(), color: color(2), dash: true, fill: false },
                ],
                opts: ChartOpts { stack: false, yfmt: YFmt::Count, ymin: Some(0.0), ymax: None },
                height: 160,
            }

            // ---- Temperature --------------------------------------------------
            if !status.temps.is_empty() {
                Chart {
                    id: "temp".to_string(),
                    title: "Temperature".to_string(),
                    cur: status
                        .temps
                        .iter()
                        .map(|t| format!("{} {}", theme::fmt_temp(t.celsius as f64), t.name))
                        .collect::<Vec<_>>()
                        .join(" · "),
                    series: status
                        .temps
                        .iter()
                        .enumerate()
                        .map(|(i, t)| Serie {
                            name: format!("temp.{}", api::sanitize(&t.name)),
                            label: t.name.clone(),
                            color: color(i),
                            dash: false,
                            fill: false,
                        })
                        .collect(),
                    opts: ChartOpts { stack: false, yfmt: YFmt::Temp, ymin: None, ymax: None },
                    height: 160,
                }
            }

            // ---- Network ------------------------------------------------------
            for n in main_ifaces {
                Chart {
                    id: format!("net-{}", api::sanitize(&n.name)),
                    title: n.name.clone(),
                    cur: format!("↓ {}  ↑ {}", theme::fmt_rate(n.rx_bps), theme::fmt_rate(n.tx_bps)),
                    series: vec![
                        Serie { name: format!("net.{}.rx", n.name), label: "rx".into(), color: color(0), dash: false, fill: true },
                        Serie { name: format!("net.{}.tx", n.name), label: "tx".into(), color: color(1), dash: true, fill: false },
                    ],
                    opts: ChartOpts { stack: false, yfmt: YFmt::Rate, ymin: Some(0.0), ymax: None },
                    height: 160,
                }
            }

            // ---- Virtual network (only if active) ------------------------------
            if !virt_ifaces.is_empty() {
                div { class: "grid-section",
                    div { class: "section-title", "virtual interfaces" }
                    for n in virt_ifaces {
                        Chart {
                            id: format!("net-{}", api::sanitize(&n.name)),
                            title: n.name.clone(),
                            cur: format!("↓ {}  ↑ {}", theme::fmt_rate(n.rx_bps), theme::fmt_rate(n.tx_bps)),
                            series: vec![
                                Serie { name: format!("net.{}.rx", n.name), label: "rx".into(), color: color(0), dash: false, fill: true },
                                Serie { name: format!("net.{}.tx", n.name), label: "tx".into(), color: color(1), dash: true, fill: false },
                            ],
                            opts: ChartOpts { stack: false, yfmt: YFmt::Rate, ymin: Some(0.0), ymax: None },
                            height: 140,
                        }
                    }
                }
            }

            // ---- Disk IO ------------------------------------------------------
            for disk in &status.disk {
                Chart {
                    id: format!("disk-{}", api::sanitize(&disk.name)),
                    title: disk.name.clone(),
                    cur: format!(
                        "↓ {}  ↑ {}  ·  {:.0}% busy",
                        theme::fmt_rate(disk.read_bps),
                        theme::fmt_rate(disk.write_bps),
                        disk.util_pct
                    ),
                    series: vec![
                        Serie { name: format!("disk.{}.read", disk.name), label: "read".into(), color: color(0), dash: false, fill: true },
                        Serie { name: format!("disk.{}.write", disk.name), label: "write".into(), color: color(1), dash: true, fill: false },
                    ],
                    opts: ChartOpts { stack: false, yfmt: YFmt::Rate, ymin: Some(0.0), ymax: None },
                    height: 160,
                }
            }

            // ---- Disk space ----------------------------------------------------
            for f in fs_cards {
                Chart {
                    id: format!("fs-{}", api::mount_series(&f.0)),
                    title: f.0.clone(),
                    cur: f.1.clone(),
                    series: f.2,
                    opts: ChartOpts { stack: true, yfmt: YFmt::Bytes, ymin: Some(0.0), ymax: Some(f.3) },
                    height: 140,
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn cpu_series(ncores: usize) -> Vec<Serie> {
    if ncores == 0 {
        vec![Serie {
            name: "cpu.total".into(),
            label: "total".into(),
            color: color(0),
            dash: false,
            fill: true,
        }]
    } else {
        (0..ncores)
            .map(|i| Serie {
                name: format!("cpu.core.{i}"),
                label: format!("c{i}"),
                color: color(i),
                dash: false,
                fill: true,
            })
            .collect()
    }
}

fn is_virtual(name: &str) -> bool {
    name == "lo"
        || name.starts_with("veth")
        || name.starts_with("podman")
        || name.starts_with("vm-")
        || name.contains("-tap")
}

fn iface_active(d: &ApiData, name: &str) -> bool {
    let Some(sd) = d.series.as_ref() else { return false };
    let peak = |key: &str| {
        sd.series
            .get(key)
            .map(|v| v.iter().flatten().fold(0.0f64, |a, b| a.max(*b)))
            .unwrap_or(0.0)
    };
    peak(&format!("net.{name}.rx")) + peak(&format!("net.{name}.tx")) > 100.0
}

/// (title, current, series, total)
fn fs_card(f: &FsStatus, d: &Option<ApiData>) -> (String, String, Vec<Serie>, f64) {
    let mount = f.mount.clone();
    let m = api::mount_series(&mount);
    let pct = if f.total > 0 {
        f.used as f64 / f.total as f64 * 100.0
    } else {
        0.0
    };
    // live pct from the series (may be more current than status)
    let live_pct = api::series_last(d, &format!("fs.{m}.used"))
        .zip(api::series_last(d, &format!("fs.{m}.avail")))
        .map(|(used, avail)| {
            let total = used + avail;
            if total > 0.0 {
                used / total * 100.0
            } else {
                0.0
            }
        })
        .unwrap_or(pct);
    let cur = format!(
        "{:.0}% used · {} free of {}",
        live_pct,
        theme::fmt_bytes(f.avail as f64),
        theme::fmt_bytes(f.total as f64)
    );
    let series = vec![
        Serie { name: format!("fs.{m}.used"), label: "used".into(), color: color(3), dash: false, fill: true },
        Serie { name: format!("fs.{m}.avail"), label: "free".into(), color: color(4), dash: false, fill: true },
    ];
    (mount, cur, series, f.total as f64)
}

#[component]
fn Footer() -> Element {
    rsx! {
        footer { class: "footer",
            "history is in-memory · resets when the service restarts · data refreshes every second"
        }
    }
}
