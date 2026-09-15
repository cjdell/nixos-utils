//! Theme colours + value formatters.

use js_sys::Date;
use wasm_bindgen::JsValue;

pub const BG: &str = "#0d1322";
pub const CARD_BG: &str = "#161e33";
pub const BORDER: &str = "#253252";
pub const GRID: &str = "rgba(148,163,184,0.10)";
pub const TEXT: &str = "#cbd5e1";
pub const MUTED: &str = "#7d8db1";
pub const AXIS: &str = "#5b6b8f";

/// Pleasant high-contrast palette, good on dark backgrounds.
pub const SERIES: [&str; 14] = [
    "#22d3ee", // cyan
    "#f472b6", // pink
    "#a78bfa", // violet
    "#fbbf24", // amber
    "#34d399", // emerald
    "#60a5fa", // blue
    "#fb7185", // rose
    "#2dd4bf", // teal
    "#818cf8", // indigo
    "#fb923c", // orange
    "#e879f9", // fuchsia
    "#38bdf8", // sky
    "#facc15", // yellow
    "#4ade80", // green
];

pub fn color(i: usize) -> String {
    SERIES[i % SERIES.len()].to_string()
}

/// "rgba(r,g,b,a)" from a #rrggbb string.
pub fn rgba(hex: &str, a: f64) -> String {
    let h = hex.trim_start_matches('#');
    if h.len() != 6 {
        return hex.to_string();
    }
    let r = u8::from_str_radix(&h[0..2], 16).unwrap_or(0);
    let g = u8::from_str_radix(&h[2..4], 16).unwrap_or(0);
    let b = u8::from_str_radix(&h[4..6], 16).unwrap_or(0);
    format!("rgba({r},{g},{b},{a})")
}

pub fn fmt_bytes(v: f64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let neg = v < 0.0;
    let mut v = v.abs();
    let mut i = 0;
    while v >= 1024.0 && i < UNITS.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    let s = if i == 0 {
        format!("{v:.0} {}", UNITS[i])
    } else {
        format!("{v:.1} {}", UNITS[i])
    };
    if neg {
        format!("-{s}")
    } else {
        s
    }
}

pub fn fmt_rate(v: f64) -> String {
    format!("{}/s", fmt_bytes(v))
}

pub fn fmt_pct(v: f64) -> String {
    format!("{v:.1}%")
}

pub fn fmt_temp(v: f64) -> String {
    format!("{v:.1}°C")
}

pub fn fmt_num(v: f64) -> String {
    format!("{v:.2}")
}

pub fn fmt_iops(v: f64) -> String {
    format!("{v:.0} iops")
}

/// Local wall-clock HH:MM[:SS].
pub fn fmt_time(ts: f64, with_secs: bool) -> String {
    let d = Date::new(&JsValue::from_f64(ts * 1000.0));
    let h = d.get_hours();
    let m = d.get_minutes();
    if with_secs {
        format!("{h:02}:{m:02}:{:02}", d.get_seconds())
    } else {
        format!("{h:02}:{m:02}")
    }
}

pub fn fmt_uptime(secs: f64) -> String {
    let d = (secs / 86400.0) as u64;
    let h = ((secs % 86400.0) / 3600.0) as u64;
    let m = ((secs % 3600.0) / 60.0) as u64;
    if d > 0 {
        format!("{d}d {h}h {m}m")
    } else if h > 0 {
        format!("{h}h {m}m")
    } else {
        format!("{m}m")
    }
}
