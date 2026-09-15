//! Canvas-based time-series chart: dark theme, gridlines, axis labels,
//! stacked/line modes, hover crosshair + tooltip. Drawn entirely in Rust
//! (dioxus 0.7 web runtime needs no external JS).

use std::collections::HashMap;

use dioxus::prelude::*;
use js_sys::Array;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement};

use crate::api::ApiData;
use crate::theme;

#[derive(Clone, Debug, PartialEq)]
pub struct Serie {
    pub name: String,
    pub label: String,
    pub color: String,
    pub dash: bool,
    pub fill: bool,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum YFmt {
    Percent,
    Bytes,
    Rate,
    Temp,
    Count,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChartOpts {
    pub stack: bool,
    pub yfmt: YFmt,
    pub ymin: Option<f64>,
    pub ymax: Option<f64>,
}

impl ChartOpts {
    pub fn new(yfmt: YFmt) -> Self {
        ChartOpts {
            stack: false,
            yfmt,
            ymin: None,
            ymax: None,
        }
    }
}

#[component]
pub fn Chart(
    id: String,
    title: String,
    cur: String,
    series: Vec<Serie>,
    opts: ChartOpts,
    height: u32,
) -> Element {
    let data = use_context::<Signal<Option<ApiData>>>();
    let hover = use_signal(|| None::<(f64, f64)>);

    // Redraw when data or hover changes. Captured values are stable clones;
    // live text comes through the `cur` prop instead.
    let s_id = id.clone();
    let s_series = series.clone();
    let _ = use_effect(move || {
        let _ = data.read();
        let h = hover();
        redraw(&s_id, &data, &s_series, opts, h);
    });

    let legend = series.iter().map(|s| {
        rsx! {
            span {
                class: "legend-item",
                style: "color: {s.color}",
                span { class: "legend-dot", style: "background: {s.color}" }
                "{s.label}"
            }
        }
    });

    rsx! {
        div { class: "card",
            div { class: "card-head",
                div { class: "card-titles",
                    div { class: "card-title", "{title}" }
                    div { class: "card-cur", "{cur}" }
                }
                div { class: "legend", {legend} }
            }
            canvas {
                id: "{id}",
                class: "chart-canvas",
                style: "height: {height}px",
                onmousemove: move |evt| {
                    let p = evt.element_coordinates();
                    let mut hover = hover;
                    hover.set(Some((p.x, p.y)));
                },
                onmouseleave: move |_| { let mut hover = hover; hover.set(None); },
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Drawing
// ---------------------------------------------------------------------------

fn get_canvas(id: &str) -> Option<(HtmlCanvasElement, CanvasRenderingContext2d)> {
    let doc = web_sys::window()?.document()?;
    let el = doc.get_element_by_id(id)?;
    let canvas = el.dyn_into::<HtmlCanvasElement>().ok()?;
    let ctx = canvas.get_context("2d").ok()??.dyn_into::<CanvasRenderingContext2d>().ok()?;
    Some((canvas, ctx))
}

fn redraw(
    id: &str,
    data: &Signal<Option<ApiData>>,
    series: &[Serie],
    opts: ChartOpts,
    hover: Option<(f64, f64)>,
) {
    let Some((canvas, ctx)) = get_canvas(id) else { return };

    let dpr = web_sys::window()
        .map(|w| w.device_pixel_ratio())
        .unwrap_or(1.0)
        .max(1.0);
    let w = canvas.client_width() as f64;
    let h = canvas.client_height() as f64;
    if w <= 1.0 || h <= 1.0 {
        return;
    }
    let bw = (w * dpr).round() as u32;
    let bh = (h * dpr).round() as u32;
    if canvas.width() != bw {
        canvas.set_width(bw);
    }
    if canvas.height() != bh {
        canvas.set_height(bh);
    }
    ctx.set_transform(dpr, 0.0, 0.0, dpr, 0.0, 0.0).ok();

    // background
    ctx.set_fill_style_str(theme::BG);
    ctx.fill_rect(0.0, 0.0, w, h);

    let d = data.read();
    let (Some(_api), Some(sd)) = (d.as_ref(), d.as_ref().and_then(|d| d.series.as_ref())) else {
        draw_empty(&ctx, w, h);
        return;
    };
    if sd.t.is_empty() || series.is_empty() {
        draw_empty(&ctx, w, h);
        return;
    }

    let from = sd.from;
    let to = sd.to;
    let span = (to - from).max(1.0);

    // ---- y range ---------------------------------------------------------
    let mut ymin = opts.ymin.unwrap_or(f64::INFINITY);
    let mut ymax = opts.ymax.unwrap_or(f64::NEG_INFINITY);
    if opts.stack {
        // cumulative per column
        let mut cum = vec![0.0f64; sd.t.len()];
        for s in series {
            if let Some(vals) = sd.series.get(&s.name) {
                for (i, v) in vals.iter().enumerate() {
                    if let Some(v) = v {
                        cum[i] += *v;
                    }
                }
            }
        }
        for v in &cum {
            ymin = ymin.min(*v);
            ymax = ymax.max(*v);
        }
    } else {
        for s in series {
            if let Some(vals) = sd.series.get(&s.name) {
                for v in vals.iter().flatten() {
                    ymin = ymin.min(*v);
                    ymax = ymax.max(*v);
                }
            }
        }
    }
    if !ymin.is_finite() {
        ymin = 0.0;
    }
    if !ymax.is_finite() {
        ymax = 1.0;
    }
    if matches!(opts.yfmt, YFmt::Percent | YFmt::Bytes | YFmt::Rate | YFmt::Count) {
        ymin = ymin.min(0.0);
    }
    if (ymax - ymin).abs() < 1e-9 {
        ymax = ymin + 1.0;
    }

    // ---- layout ----------------------------------------------------------
    let pad_l = 8.0;
    let pad_r = 58.0;
    let pad_t = 8.0;
    let pad_b = 22.0;
    let pw = (w - pad_l - pad_r).max(10.0);
    let ph = (h - pad_t - pad_b).max(10.0);
    let x = |tv: f64| pad_l + (tv - from) / span * pw;
    let y = |v: f64| pad_t + ph - (v - ymin) / (ymax - ymin) * ph;

    ctx.set_font("11px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace");
    ctx.set_line_width(1.0);

    // ---- grid + y labels -------------------------------------------------
    let step = nice_step(ymax - ymin, 5.0);
    let first = (ymin / step).ceil() * step;
    let mut i = 0.0;
    loop {
        let v = first + i * step;
        if v > ymax + 1e-9 {
            break;
        }
        let gy = y(v);
        ctx.set_stroke_style_str(theme::GRID);
        ctx.begin_path();
        ctx.move_to(pad_l, gy);
        ctx.line_to(w - pad_r, gy);
        ctx.stroke();
        ctx.set_fill_style_str(theme::AXIS);
        ctx.set_text_align("right");
        ctx.set_text_baseline("middle");
        ctx.fill_text(&fmt_y(v, opts.yfmt), w - pad_r + 4.0, gy).ok();
        i += 1.0;
    }

    // ---- x labels ---------------------------------------------------------
    let with_secs = span <= 15.0 * 60.0;
    ctx.set_text_align("center");
    ctx.set_text_baseline("top");
    for k in 0..=3usize {
        let tv = from + span * (k as f64 / 3.0);
        let gx = x(tv);
        ctx.set_stroke_style_str(theme::GRID);
        ctx.begin_path();
        ctx.move_to(gx, pad_t);
        ctx.line_to(gx, h - pad_b);
        ctx.stroke();
        ctx.set_fill_style_str(theme::AXIS);
        ctx.fill_text(&theme::fmt_time(tv, with_secs), gx, h - pad_b + 6.0).ok();
    }

    // ---- series ------------------------------------------------------------
    if opts.stack {
        draw_stacked(&ctx, series, &sd.series, &sd.t, &x, &y);
    } else {
        for s in series {
            draw_line(&ctx, s, &sd.series, &sd.t, &x, &y, pad_l, pad_t + ph);
        }
    }

    // ---- hover --------------------------------------------------------------
    if let Some((mx, my)) = hover {
        if mx >= pad_l && mx <= w - pad_r && my >= pad_t && my <= pad_t + ph {
            draw_hover(&ctx, series, &sd, &x, pad_l, w, h, mx, my, opts.yfmt);
        }
    }
}

fn draw_empty(ctx: &CanvasRenderingContext2d, w: f64, h: f64) {
    ctx.set_fill_style_str(theme::MUTED);
    ctx.set_font("12px ui-monospace, monospace");
    ctx.set_text_align("center");
    ctx.set_text_baseline("middle");
    ctx.fill_text("collecting…", w / 2.0, h / 2.0).ok();
}

fn draw_line(
    ctx: &CanvasRenderingContext2d,
    s: &Serie,
    map: &HashMap<String, Vec<Option<f64>>>,
    t: &[f64],
    x: &dyn Fn(f64) -> f64,
    y: &dyn Fn(f64) -> f64,
    _pad_l: f64,
    baseline: f64,
) {
    let Some(vals) = map.get(&s.name) else { return };
    // collect points
    let mut pts: Vec<(f64, f64)> = Vec::new();
    for (j, tv) in t.iter().enumerate() {
        let Some(v) = vals.get(j).copied().flatten() else { continue };
        pts.push((x(*tv), y(v)));
    }
    if pts.len() < 2 {
        return;
    }

    ctx.set_line_dash(&Array::new());
    if s.fill {
        // area fill
        let grad = ctx.create_linear_gradient(0.0, 0.0, 0.0, baseline);
        {
            grad.add_color_stop(0.0, &theme::rgba(&s.color, 0.28));
            grad.add_color_stop(1.0, &theme::rgba(&s.color, 0.02));
            ctx.set_fill_style_canvas_gradient(&grad);
            ctx.begin_path();
            ctx.move_to(pts[0].0, baseline);
            for (px, py) in &pts {
                ctx.line_to(*px, *py);
            }
            let last = pts[pts.len() - 1];
            ctx.line_to(last.0, baseline);
            ctx.close_path();
            ctx.fill();
        }
    }

    ctx.set_stroke_style_str(&s.color);
    if s.dash {
        let arr = Array::new_with_length(2);
        arr.set(0, JsValue::from_f64(4.0));
        arr.set(1, JsValue::from_f64(4.0));
        ctx.set_line_dash(&arr);
    } else {
        ctx.set_line_dash(&Array::new());
    }
    ctx.set_line_width(1.5);
    ctx.begin_path();
    ctx.move_to(pts[0].0, pts[0].1);
    for (px, py) in pts.iter().skip(1) {
        ctx.line_to(*px, *py);
    }
    ctx.stroke();
    ctx.set_line_dash(&Array::new());
}

fn draw_stacked(
    ctx: &CanvasRenderingContext2d,
    series: &[Serie],
    map: &HashMap<String, Vec<Option<f64>>>,
    t: &[f64],
    x: &dyn Fn(f64) -> f64,
    y: &dyn Fn(f64) -> f64,
) {
    let n = t.len();
    let mut cum = vec![0.0f64; n];
    for s in series {
        let Some(vals) = map.get(&s.name) else { continue };
        let mut bottom = Vec::with_capacity(n);
        let mut top = Vec::with_capacity(n);
        for (j, tv) in t.iter().enumerate() {
            let v = vals.get(j).copied().flatten().unwrap_or(0.0);
            bottom.push((x(*tv), y(cum[j])));
            cum[j] += v;
            top.push((x(*tv), y(cum[j])));
        }
        // filled polygon: top forward, then down the right edge and back
        // along the bottom (previous cumulative) edge
        ctx.begin_path();
        ctx.move_to(top[0].0, top[0].1);
        for p in top.iter().skip(1) {
            ctx.line_to(p.0, p.1);
        }
        ctx.line_to(bottom[n - 1].0, bottom[n - 1].1);
        for p in bottom.iter().rev() {
            ctx.line_to(p.0, p.1);
        }
        ctx.close_path();
        ctx.set_fill_style_str(&theme::rgba(&s.color, 0.7));
        ctx.fill();
        // top edge stroke
        ctx.set_stroke_style_str(&s.color);
        ctx.set_line_width(1.2);
        ctx.begin_path();
        ctx.move_to(top[0].0, top[0].1);
        for p in top.iter().skip(1) {
            ctx.line_to(p.0, p.1);
        }
        ctx.stroke();
    }
}

fn draw_hover(
    ctx: &CanvasRenderingContext2d,
    series: &[Serie],
    sd: &crate::api::Series,
    x: &dyn Fn(f64) -> f64,
    pad_l: f64,
    w: f64,
    h: f64,
    mx: f64,
    my: f64,
    yfmt: YFmt,
) {
    // nearest sample index
    let mut best = 0usize;
    let mut best_d = f64::INFINITY;
    for (i, tv) in sd.t.iter().enumerate() {
        let d = (x(*tv) - mx).abs();
        if d < best_d {
            best_d = d;
            best = i;
        }
    }
    if best >= sd.t.len() {
        return;
    }
    let tv = sd.t[best];
    let gx = x(tv);

    // crosshair
    ctx.set_stroke_style_str(theme::MUTED);
    ctx.set_line_width(1.0);
    let dash = Array::new_with_length(2);
    dash.set(0, JsValue::from_f64(3.0));
    dash.set(1, JsValue::from_f64(3.0));
    ctx.set_line_dash(&dash);
    ctx.begin_path();
    ctx.move_to(gx, 0.0);
    ctx.line_to(gx, h);
    ctx.stroke();
    ctx.set_line_dash(&Array::new());

    // tooltip contents
    let mut lines: Vec<(String, String, String)> = Vec::new(); // (label, value, color)
    for s in series {
        if let Some(v) = sd.series.get(&s.name).and_then(|vals| vals.get(best).copied().flatten())
        {
            lines.push((s.label.clone(), fmt_y(v, yfmt), s.color.clone()));
        }
    }
    if lines.is_empty() {
        return;
    }

    ctx.set_font("11px ui-monospace, SFMono-Regular, Menlo, Consolas, monospace");
    let mut widest = 0.0f64;
    for (l, v, _) in &lines {
        let text = format!("{l} {v}");
        if let Ok(m) = ctx.measure_text(&text) {
            widest = widest.max(m.width());
        }
    }
    let box_w = widest + 20.0;
    let box_h = lines.len() as f64 * 15.0 + 26.0;
    let mut bx = gx + 12.0;
    let mut by = my + 12.0;
    if bx + box_w > w - 8.0 {
        bx = gx - box_w - 12.0;
    }
    if by + box_h > h - 8.0 {
        by = h - box_h - 8.0;
    }
    if bx < pad_l {
        bx = pad_l;
    }

    ctx.set_fill_style_str("rgba(10,15,28,0.92)");
    ctx.set_stroke_style_str(theme::BORDER);
    ctx.set_line_width(1.0);
    ctx.begin_path();
    ctx.round_rect(bx, by, box_w, box_h).ok();
    ctx.fill();
    ctx.stroke();

    ctx.set_fill_style_str(theme::TEXT);
    ctx.set_text_align("left");
    ctx.set_text_baseline("top");
    ctx.fill_text(&theme::fmt_time(tv, true), bx + 10.0, by + 8.0).ok();
    for (i, (l, v, color)) in lines.iter().enumerate() {
        let ty = by + 24.0 + i as f64 * 15.0;
        ctx.set_fill_style_str(theme::MUTED);
        ctx.fill_text(l, bx + 10.0, ty).ok();
        let vx = bx + 10.0 + ctx.measure_text(l).map(|m| m.width()).unwrap_or(0.0) + 8.0;
        ctx.set_fill_style_str(color);
        ctx.fill_text(v, vx, ty).ok();
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fmt_y(v: f64, fmt: YFmt) -> String {
    match fmt {
        YFmt::Percent => format!("{v:.0}%", ),
        YFmt::Bytes => theme::fmt_bytes(v),
        YFmt::Rate => theme::fmt_rate(v),
        YFmt::Temp => format!("{v:.0}°", ),
        YFmt::Count => format!("{v:.0}", ),
    }
}

fn nice_step(range: f64, target: f64) -> f64 {
    if range <= 0.0 {
        return 1.0;
    }
    let raw = range / target;
    let mag = 10f64.powf(raw.log10().floor());
    let norm = raw / mag;
    let step = if norm < 1.5 {
        1.0
    } else if norm < 3.0 {
        2.0
    } else if norm < 7.0 {
        5.0
    } else {
        10.0
    };
    step * mag
}
