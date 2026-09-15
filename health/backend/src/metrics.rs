//! System metrics collection from `/proc` + `statvfs` + hwmon.
//!
//! A `Sampler` runs a background loop that reads the kernel's virtual files
//! every `interval` and keeps two ring buffers:
//!
//! - **fast**: every sample, holds the last `fast_cap` samples (1s × 1h by
//!   default) — used for the short, high-resolution time ranges.
//! - **slow**: every `slow_every`-th sample, holds `slow_cap` (10s × 24h by
//!   default) — used for the long ranges.
//!
//! All rates (CPU %, network bytes/s, disk IOPS…) are computed as deltas
//! between consecutive samples. Everything is in-memory; a restart resets the
//! history.

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, Default)]
pub struct MemInfo {
    pub total: u64,
    pub available: u64,
    pub cache: u64,
    pub buffers: u64,
    pub free: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

impl MemInfo {
    /// What we report as "used": everything that is not available for new apps.
    pub fn used(&self) -> u64 {
        self.total.saturating_sub(self.available)
    }
}

#[derive(Clone, Debug)]
pub struct NetRate {
    pub name: String,
    pub rx_bps: f64,
    pub tx_bps: f64,
}

#[derive(Clone, Debug)]
pub struct DiskRate {
    pub name: String,
    pub read_bps: f64,
    pub write_bps: f64,
    pub read_iops: f64,
    pub write_iops: f64,
    /// Fraction of the interval the device was busy doing IO (0-100).
    pub util_pct: f32,
}

#[derive(Clone, Debug)]
pub struct FsUsage {
    pub device: String,
    pub mount: String,
    pub fstype: String,
    pub total: u64,
    pub used: u64,
    pub avail: u64,
}

#[derive(Clone, Debug)]
pub struct TempReading {
    pub name: String,
    pub celsius: f32,
}

/// One captured point in time.
#[derive(Clone, Debug)]
pub struct Sample {
    pub t: f64,
    pub cpu_total: f32,
    pub cpu_cores: Vec<f32>,
    pub mem: MemInfo,
    pub net: Vec<NetRate>,
    pub disk: Vec<DiskRate>,
    pub fs: Vec<FsUsage>,
    pub load: [f32; 3],
    pub temps: Vec<TempReading>,
    pub uptime: f64,
}

// ---------------------------------------------------------------------------
// Previous-value state used to compute deltas
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct PrevCpu {
    idle: u64,
    total: u64,
}

#[derive(Default, Clone)]
struct PrevDisk {
    reads: u64,
    writes: u64,
    read_sectors: u64,
    write_sectors: u64,
    io_ms: u64,
}

#[derive(Default)]
struct Prev {
    cpu: HashMap<String, PrevCpu>,
    net: HashMap<String, (u64, u64)>,
    disk: HashMap<String, PrevDisk>,
}

// ---------------------------------------------------------------------------
// Raw /proc readers
// ---------------------------------------------------------------------------

/// (name, idle_ticks, total_ticks) for the `cpu` line and every `cpuN` line.
fn read_cpu_times() -> Vec<(String, u64, u64)> {
    let mut out = Vec::new();
    let Ok(text) = fs::read_to_string("/proc/stat") else {
        return out;
    };
    for line in text.lines() {
        if !line.starts_with("cpu") {
            continue;
        }
        let mut it = line.split_whitespace();
        let name = it.next().unwrap_or_default().to_string();
        let vals: Vec<u64> = it.filter_map(|v| v.parse().ok()).collect();
        if vals.len() < 4 {
            continue;
        }
        // idle = idle + iowait (the kernel considers iowait as idle)
        let idle = vals[3] + vals.get(4).copied().unwrap_or(0);
        let total: u64 = vals.iter().sum();
        out.push((name, idle, total));
    }
    out
}

fn read_meminfo() -> MemInfo {
    let mut m = MemInfo::default();
    let Ok(text) = fs::read_to_string("/proc/meminfo") else {
        return m;
    };
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let Some(key) = it.next() else { continue };
        let Some(Ok(val)) = it.next().map(|v| v.parse::<u64>()) else {
            continue;
        };
        let kb = val * 1024;
        match key {
            "MemTotal:" => m.total = kb,
            "MemAvailable:" => m.available = kb,
            "MemFree:" => m.free = kb,
            "Buffers:" => m.buffers = kb,
            "Cached:" => m.cache = kb,
            "SwapTotal:" => m.swap_total = kb,
            "SwapFree:" => m.swap_used = m.swap_total.saturating_sub(kb),
            _ => {}
        }
    }
    m
}

/// (name, rx_bytes, tx_bytes) per interface.
fn read_net_dev() -> Vec<(String, u64, u64)> {
    let mut out = Vec::new();
    let Ok(text) = fs::read_to_string("/proc/net/dev") else {
        return out;
    };
    for line in text.lines().skip(2) {
        let Some((name, rest)) = line.split_once(':') else {
            continue;
        };
        let fields: Vec<u64> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
        if fields.len() < 9 {
            continue;
        }
        out.push((name.trim().to_string(), fields[0], fields[8]));
    }
    out
}

#[derive(Default, Clone)]
struct DiskCounters {
    name: String,
    reads: u64,
    writes: u64,
    read_sectors: u64,
    write_sectors: u64,
    io_ms: u64,
}

fn read_diskstats() -> Vec<DiskCounters> {
    let mut out = Vec::new();
    let Ok(text) = fs::read_to_string("/proc/diskstats") else {
        return out;
    };
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.len() < 14 {
            continue;
        }
        let major: u64 = f[0].parse().unwrap_or(0);
        // skip loop devices (major 7) — they are always idle and noisy
        if major == 7 {
            continue;
        }
        let name = f[2].to_string();
        let num = |i: usize| f.get(i).and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
        out.push(DiskCounters {
            name,
            reads: num(3),
            writes: num(7),
            read_sectors: num(5),
            write_sectors: num(9),
            io_ms: num(12),
        });
    }
    out
}

/// (device, mountpoint, fstype) for every mount whose source is a real block
/// device (skips proc/sysfs/tmpfs/overlay and loop devices).
fn read_mounts() -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let Ok(text) = fs::read_to_string("/proc/mounts") else {
        return out;
    };
    for line in text.lines() {
        let mut it = line.split_whitespace();
        let (Some(dev), Some(mnt), Some(fst)) = (it.next(), it.next(), it.next()) else {
            continue;
        };
        if !dev.starts_with("/dev/") || dev.starts_with("/dev/loop") {
            continue;
        }
        out.push((dev.to_string(), mnt.to_string(), fst.to_string()));
    }
    out
}

struct StatVfs {
    blocks: u64,
    bfree: u64,
    bavail: u64,
    frsize: u64,
}

fn statvfs(path: &str) -> Option<StatVfs> {
    let c = std::ffi::CString::new(path).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } == 0 {
        Some(StatVfs {
            blocks: st.f_blocks as u64,
            bfree: st.f_bfree as u64,
            bavail: st.f_bavail as u64,
            frsize: st.f_frsize as u64,
        })
    } else {
        None
    }
}

fn read_fs_usage() -> Vec<FsUsage> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for (dev, mnt, fst) in read_mounts() {
        if !seen.insert(dev.clone()) {
            continue; // bind-mounts / subvolumes of the same device
        }
        let Some(st) = statvfs(&mnt) else { continue };
        if st.frsize == 0 || st.blocks == 0 {
            continue;
        }
        let total = st.blocks.saturating_mul(st.frsize);
        let used = st.blocks.saturating_sub(st.bfree).saturating_mul(st.frsize);
        let avail = st.bavail.saturating_mul(st.frsize);
        out.push(FsUsage {
            device: dev,
            mount: mnt,
            fstype: fst,
            total,
            used,
            avail,
        });
    }
    out
}

fn read_loadavg() -> Option<[f32; 3]> {
    let text = fs::read_to_string("/proc/loadavg").ok()?;
    let mut it = text.split_whitespace();
    Some([
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
        it.next()?.parse().ok()?,
    ])
}

fn read_uptime() -> f64 {
    fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|s| s.split_whitespace().next().map(|v| v.parse::<f64>().unwrap_or(0.0)))
        .unwrap_or(0.0)
}

/// Best-effort temperature sensors from `/sys/class/hwmon/*`.
fn read_temps() -> Vec<TempReading> {
    let mut out = Vec::new();
    let Ok(hwmons) = fs::read_dir("/sys/class/hwmon") else {
        return out;
    };
    for entry in hwmons.flatten() {
        let dir = entry.path();
        let chip = fs::read_to_string(dir.join("name")).unwrap_or_default();
        let chip = chip.trim().to_string();
        let mut files: Vec<String> = Vec::new();
        if let Ok(rd) = fs::read_dir(&dir) {
            for e in rd.flatten() {
                if let Some(f) = e.file_name().to_str().map(|s| s.to_string()) {
                    files.push(f);
                }
            }
        }
        files.sort();
        for f in files {
            let Some(idx) = f.strip_prefix("temp").and_then(|s| s.strip_suffix("_input")) else {
                continue;
            };
            let Ok(raw) = fs::read_to_string(dir.join(&f)) else {
                continue;
            };
            let Ok(milli) = raw.trim().parse::<i64>() else {
                continue;
            };
            let label = fs::read_to_string(dir.join(format!("temp{idx}_label"))).unwrap_or_default();
            let label = label.trim().to_string();
            let name = if label.is_empty() {
                format!("{chip} {idx}")
            } else {
                label
            };
            out.push(TempReading {
                name,
                celsius: milli as f32 / 1000.0,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Sampling
// ---------------------------------------------------------------------------

fn now() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

fn collect(prev: &mut Prev, dt: f64) -> Sample {
    // CPU
    let mut cpu_total = 0.0f32;
    let mut cpu_cores = Vec::new();
    for (name, idle, total) in read_cpu_times() {
        let (pidle, ptotal) = prev
            .cpu
            .get(&name)
            .map(|p| (p.idle, p.total))
            .unwrap_or((idle, total));
        let pct = if total > ptotal {
            let d_idle = idle.saturating_sub(pidle);
            let d_total = total - ptotal;
            ((1.0 - d_idle as f64 / d_total as f64) * 100.0) as f32
        } else {
            0.0
        };
        prev.cpu.insert(name.clone(), PrevCpu { idle, total });
        if name == "cpu" {
            cpu_total = pct;
        } else if let Some(rest) = name.strip_prefix("cpu") {
            // name is "cpu0", "cpu1", …
            if let Ok(i) = rest.parse::<usize>() {
                while cpu_cores.len() <= i {
                    cpu_cores.push(0.0);
                }
                cpu_cores[i] = pct;
            }
        }
    }

    // Network
    let mut net = Vec::new();
    for (name, rx, tx) in read_net_dev() {
        let (prx, ptx) = prev.net.get(&name).copied().unwrap_or((rx, tx));
        let rx_bps = rx.saturating_sub(prx) as f64 / dt;
        let tx_bps = tx.saturating_sub(ptx) as f64 / dt;
        prev.net.insert(name.clone(), (rx, tx));
        net.push(NetRate { name, rx_bps, tx_bps });
    }

    // Disk IO
    let mut disk = Vec::new();
    for c in read_diskstats() {
        let p = prev.disk.get(&c.name).cloned().unwrap_or_else(|| {
            // first sighting: treat previous counters as equal so the first
            // sample shows 0 activity instead of a huge spike
            PrevDisk {
                reads: c.reads,
                writes: c.writes,
                read_sectors: c.read_sectors,
                write_sectors: c.write_sectors,
                io_ms: c.io_ms,
            }
        });
        let dr = c.reads.saturating_sub(p.reads);
        let dw = c.writes.saturating_sub(p.writes);
        let dsec_r = c.read_sectors.saturating_sub(p.read_sectors);
        let dsec_w = c.write_sectors.saturating_sub(p.write_sectors);
        let dio_ms = c.io_ms.saturating_sub(p.io_ms);
        prev.disk.insert(c.name.clone(), PrevDisk {
            reads: c.reads,
            writes: c.writes,
            read_sectors: c.read_sectors,
            write_sectors: c.write_sectors,
            io_ms: c.io_ms,
        });
        let sector = 512.0;
        disk.push(DiskRate {
            name: c.name,
            read_bps: dsec_r as f64 * sector / dt,
            write_bps: dsec_w as f64 * sector / dt,
            read_iops: dr as f64 / dt,
            write_iops: dw as f64 / dt,
            util_pct: ((dio_ms as f64 / (dt * 1000.0)) * 100.0).min(100.0) as f32,
        });
    }

    let mem = read_meminfo();
    let fs_usage = read_fs_usage();
    let load = read_loadavg().unwrap_or([0.0, 0.0, 0.0]);
    let temps = read_temps();

    Sample {
        t: now(),
        cpu_total,
        cpu_cores,
        mem,
        net,
        disk,
        fs: fs_usage,
        load,
        temps,
        uptime: read_uptime(),
    }
}

// ---------------------------------------------------------------------------
// Sampler
// ---------------------------------------------------------------------------

pub struct Sampler {
    interval: Duration,
    fast_cap: usize,
    slow_every: u64,
    slow_cap: usize,
    fast: Mutex<VecDeque<Sample>>,
    slow: Mutex<VecDeque<Sample>>,
    last: Mutex<Option<Sample>>,
    prev: Mutex<Prev>,
    tick: AtomicU64,
}

impl Sampler {
    pub fn new(interval: Duration, fast_cap: usize, slow_every: u64, slow_cap: usize) -> Arc<Self> {
        Arc::new(Sampler {
            interval,
            fast_cap,
            slow_every,
            slow_cap,
            fast: Mutex::new(VecDeque::new()),
            slow: Mutex::new(VecDeque::new()),
            last: Mutex::new(None),
            prev: Mutex::new(Prev::default()),
            tick: AtomicU64::new(0),
        })
    }

    /// Background sampling loop. Runs forever.
    pub async fn run(self: &Arc<Self>) {
        loop {
            tokio::time::sleep(self.interval).await;
            let tick = self.tick.fetch_add(1, Ordering::Relaxed) + 1;
            let dt = self.interval.as_secs_f64();
            let sample = {
                let mut prev = self.prev.lock().await;
                collect(&mut prev, dt)
            };
            *self.last.lock().await = Some(sample.clone());
            {
                let mut fast = self.fast.lock().await;
                fast.push_back(sample.clone());
                while fast.len() > self.fast_cap {
                    fast.pop_front();
                }
            }
            if tick % self.slow_every == 0 {
                let mut slow = self.slow.lock().await;
                slow.push_back(sample);
                while slow.len() > self.slow_cap {
                    slow.pop_front();
                }
            }
        }
    }

    pub async fn last(&self) -> Option<Sample> {
        self.last.lock().await.clone()
    }

    /// Return one entry per time bucket in `[from, to]`, using the best ring
    /// for the requested span. Empty buckets yield `None`; the last sample of
    /// each bucket wins (long ranges therefore look stepped, which is fine).
    pub async fn query(&self, from: f64, to: f64, max_points: usize) -> Vec<Option<Sample>> {
        let span = (to - from).max(0.0);
        let fast_span = self.fast_cap as f64 * self.interval.as_secs_f64();
        let (ring, step) = if span <= fast_span {
            (self.fast.lock().await, self.interval.as_secs_f64())
        } else {
            (self.slow.lock().await, self.interval.as_secs_f64() * self.slow_every as f64)
        };
        let samples: Vec<Sample> = ring
            .iter()
            // include one sample before `from` so the first bucket has data
            .filter(|s| s.t >= from - step && s.t <= to)
            .cloned()
            .collect();

        let n = ((span / step).ceil() as usize).clamp(1, max_points);
        let bucket = if n > 0 { span / n as f64 } else { 1.0 };
        let mut out: Vec<Option<Sample>> = vec![None; n];
        for s in samples {
            if s.t < from {
                continue;
            }
            let i = (((s.t - from) / bucket) as usize).min(n - 1);
            out[i] = Some(s);
        }
        out
    }

    pub fn interval_secs(&self) -> f64 {
        self.interval.as_secs_f64()
    }
}

/// Flatten a sample into (series-name, value) pairs. Series naming is part of
/// the public API contract between the backend and the SPA.
pub fn series_of(s: &Sample) -> Vec<(String, f64)> {
    let mut v: Vec<(String, f64)> = Vec::new();
    v.push(("cpu.total".into(), s.cpu_total as f64));
    for (i, c) in s.cpu_cores.iter().enumerate() {
        v.push((format!("cpu.core.{i}"), *c as f64));
    }
    v.push(("mem.used".into(), s.mem.used() as f64));
    v.push(("mem.available".into(), s.mem.available as f64));
    v.push(("mem.cache".into(), s.mem.cache as f64));
    v.push(("mem.buffers".into(), s.mem.buffers as f64));
    v.push(("mem.free".into(), s.mem.free as f64));
    v.push(("mem.swap".into(), s.mem.swap_used as f64));
    for n in &s.net {
        v.push((format!("net.{}.rx", n.name), n.rx_bps));
        v.push((format!("net.{}.tx", n.name), n.tx_bps));
    }
    for d in &s.disk {
        v.push((format!("disk.{}.read", d.name), d.read_bps));
        v.push((format!("disk.{}.write", d.name), d.write_bps));
        v.push((format!("disk.{}.read.iops", d.name), d.read_iops));
        v.push((format!("disk.{}.write.iops", d.name), d.write_iops));
        v.push((format!("disk.{}.util", d.name), d.util_pct as f64));
    }
    for f in &s.fs {
        v.push((format!("fs.{}.used", sanitize(&f.mount)), f.used as f64));
        v.push((format!("fs.{}.avail", sanitize(&f.mount)), f.avail as f64));
    }
    v.push(("load.1".into(), s.load[0] as f64));
    v.push(("load.5".into(), s.load[1] as f64));
    v.push(("load.15".into(), s.load[2] as f64));
    for t in &s.temps {
        v.push((format!("temp.{}", sanitize(&t.name)), t.celsius as f64));
    }
    v
}

/// Keep series names URL/config friendly: anything not alphanumeric, `.`, `-`
/// or `_` becomes `-`.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Host facts read once at startup.
pub fn host_info() -> (String, String) {
    let hostname = fs::read_to_string("/proc/sys/kernel/hostname")
        .unwrap_or_default()
        .trim()
        .to_string();
    let kernel = fs::read_to_string("/proc/sys/kernel/osrelease")
        .unwrap_or_default()
        .trim()
        .to_string();
    (hostname, kernel)
}

/// Recursively list a directory (used by dev tooling / tests).
#[allow(dead_code)]
fn _dir_entries(path: &Path) -> Vec<String> {
    fs::read_dir(path)
        .map(|rd| rd.flatten().filter_map(|e| e.file_name().to_str().map(String::from)).collect())
        .unwrap_or_default()
}
