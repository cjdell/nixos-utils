# health — realtime system health dashboard

A self-contained health/status page: a Rust (axum) backend that samples the
host's `/proc` + `statvfs` + hwmon sensors and serves a dioxus SPA with
canvas-drawn realtime graphs and selectable time ranges.

Deploy anywhere that shares this repo:

```nix
# flake.nix
inputs.nixos-utils.url = "github:your/nixos-utils";

# configuration.nix
{
  imports = [ nixos-utils.nixosModules.health ];
  services.health.enable = true;
  # defaults: 127.0.0.1:8092, 1s samples, 1h @ 1s + 24h @ 10s in-memory
}
```

Add your own nginx virtual host (this module only runs the HTTP server).
For an SSO-protected example see `hosts/grafton-router/services/health.nix`
in the `nixos-config` repo.

## What it shows

- **CPU** — stacked per-core usage, current total %
- **Memory** — stacked used / cache / buffers / free (+ swap if present)
- **Load average** — 1 / 5 / 15 min
- **Temperature** — every hwmon sensor (`/sys/class/hwmon`)
- **Network** — rx/tx bytes/s per interface (virtual interfaces like
  `veth*`/`podman0` are only shown while active)
- **Disk IO** — read/write bytes/s per block device (+ busy %)
- **Disk space** — stacked used/free per filesystem

Time ranges: 1m / 5m / 15m / 1h / 6h / 24h. Hover a chart for a crosshair +
values tooltip. The SPA polls `/api/data` every second.

## Architecture

```
health/
  backend/   Rust (axum): /proc + statvfs + hwmon samplers, two ring buffers
             (1s × 1h, 10s × 24h), /api/data + /api/status, serves the SPA
  web/       dioxus 0.7 SPA: canvas charts, no external JS, no chart lib
  modules/health.nix  NixOS module (services.health.*)
```

Series naming (API contract): `cpu.total`, `cpu.core.N`, `mem.used`,
`mem.cache`, `mem.buffers`, `mem.free`, `mem.swap`, `net.<iface>.rx/.tx`,
`disk.<dev>.read/.write/.read.iops/.write.iops/.util`, `fs.<mount>.used/.avail`
(mount sanitized: `/` → `root`), `load.1/.5/.15`, `temp.<name>`.

All history is **in-memory** — a service restart resets the graphs.

## API

- `GET /api/data?from=<unix_sec>&to=<unix_sec>` — current status + downsampled
  series (`{ status: {...}, series: { from, to, step, t, series } }`).
  Series values are `null` where a bucket has no sample.
- `GET /api/status` — last sample only.
- `GET /api/health` — `ok`.
- anything else — the SPA (index.html fallback).

## Building

```bash
nix build .#health-web      # dioxus SPA → wasm + assets
nix build .#health-backend  # axum server
nix build .#health          # combined: bin/health-backend + share/health
```

The SPA build pins `wasm-bindgen =0.2.121`, `js-sys =0.3.98`,
`web-sys =0.3.98`, `wasm-bindgen-futures =0.4.71` in `web/Cargo.toml` to match
the `wasm-bindgen-cli` version in nixpkgs (the CLI and the crate's schema
version must match exactly). If you bump nixpkgs, update all four together.

Note: the flake pins `nixpkgs` at nixos-26.05 — the crates.io API download
endpoint (used by older nixpkgs' `importCargoLock`) rate-limits and returns
403s; 26.05 fetches crates from `static.crates.io` instead.

## Development

```bash
# backend
cd health/backend
nix shell nixpkgs#rustc nixpkgs#cargo nixpkgs#gcc --command cargo run

# frontend (iterate quickly)
cd health/web
nix shell nixpkgs#rustc nixpkgs#cargo nixpkgs#gcc nixpkgs#lld nixpkgs#wasm-bindgen-cli \
  --command sh -c '
    cargo build --release --target wasm32-unknown-unknown &&
    wasm-bindgen --target web --out-dir dist --no-typescript \
      target/wasm32-unknown-unknown/release/health-ui.wasm &&
    cp index.html style.css dist/'

# then point the backend at it:
LISTEN_ADDR=127.0.0.1:8092 STATIC_DIR=../web/dist ./target/release/health-backend
```

(nixpkgs' rustc ships the `wasm32-unknown-unknown` std; `lld` provides the
`wasm-ld` linker that rustc needs for wasm targets.)
