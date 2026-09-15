# AGENTS.md — Notes for AI agents working on this repository

Operational knowledge for this repo: NixOS modules, helper scripts, and
`container-ui`/`health`, Rust web apps. Everything deploys to
`grafton-router` through the `nixos-config` repo.

## Deployment loop (read this before building)

`nixos-config` consumes this repo as a **path flake input pinned by
`narHash`** in its `flake.lock`. Edits in this working tree are invisible to
that build until you re-lock over there:

```bash
cd /home/cjdell/nixos-config
nix flake lock --update-input nixos-utils   # pick up this working tree
sudo nixos-rebuild build --flake .
./scripts/switch.sh                         # or boot.sh
sudo nixos-confirm                          # grafton-router auto-rolls back after 5 min
```

- **A dirty git tree is the normal state.** The user deploys from the working
  tree; untracked directories (e.g. `container-ui/`) and uncommitted changes
  are expected, and untracked files do get built. Leave git state (commits,
  stashes, cleans) alone unless explicitly asked.
- `nixos-config`'s flake currently points at this working tree via a
  temporary absolute path input; once the changes here are pushed, that
  input is restored to the `github:` URL and re-locked.

## container-ui (Rust, axum)

Server-rendered UI for the root podman containers on grafton-router
(`container-ui.service`, binds `127.0.0.1:8091`, Kanidm OIDC SSO via nginx —
the client config lives in `nixos-config`).

- **The app has no container API client — it shells out to `podman` and
  `systemctl`** (every invocation is in `src/podman.rs`).
- **Verify podman invocations by running them as root on the machine.**
  Flag behavior varies by version (podman 5.8 here); the machine is the
  source of truth. Known incompatibility: `podman stats` rejects `--all`
  combined with an explicit container name.
- **A `—` in the UI means a command failed.** Handlers map podman errors to
  `None` (`.ok().flatten()`), which renders as `—`/empty. When a value is
  missing, reproduce the exact podman command as root instead of guessing
  from the code.
- New failure paths should be logged (`tracing::warn!`) or surfaced to the
  user — a silent default hides the failure.
- **axum gotcha:** HTML responses must go through `html_resp` (wraps
  `axum::response::Html`). A bare `(status, String)` is served as
  `text/plain`, so the browser shows raw source and inline JS never runs.
- Sessions + PKCE state are in-memory: a service restart logs everyone out.

## health (Rust, axum backend + dioxus SPA)

Realtime system health dashboard (`health.service`, binds `127.0.0.1:8092`,
SSO-protected at `health.home.chrisdell.info` via nginx). Backend in
`health/backend/` samples `/proc` + `statvfs` + hwmon every second into two
in-memory ring buffers (1s × 1h, 10s × 24h); SPA in `health/web/` is a
dioxus 0.7 app with canvas charts (no external JS, no chart lib).

- **History is in-memory** — a service restart resets the graphs (by design).
- **Series names are an API contract** between backend and SPA
  (`cpu.core.N`, `net.<iface>.rx/.tx`, `disk.<dev>.read/.write`, … — see
  `health/README.md`). If you rename a series in the backend, update the SPA.
- **Wasm build pins wasm-bindgen to 0.2.121** (matching the nixpkgs
  `wasm-bindgen-cli`): `web/Cargo.toml` pins wasm-bindgen 0.2.121, js-sys
  0.3.98, web-sys 0.3.98, wasm-bindgen-futures 0.4.71. The CLI and crate
  schema versions must match exactly — bump all four together.
- **The flake pins nixpkgs at nixos-26.05.** Older nixpkgs' `importCargoLock`
  downloads crates from the crates.io *API* (`/api/v1/crates/...`), which
  rate-limits and returns 403s; 26.05 fetches from `static.crates.io`.
- Building the SPA needs `pkgs.lld` on PATH (provides `wasm-ld`, which
  nixpkgs rustc requires for wasm targets) and the `wasm32-unknown-unknown`
  std (ships with nixpkgs rustc).
- The NixOS module is `modules/health.nix` (`services.health.*`); it only
  runs the HTTP server — nginx vhosts are added by the deploying host.

## Modules & scripts

- `modules/` — NixOS modules, consumed as
  `nixos-utils.nixosModules.{containers,health,notifications,rollback}`.
- `nu/` — nushell helpers (`list-containers`, `update-containers`), exposed
  as flake packages and wired up by the containers module.
- `deno/notify.ts` — push notification gateway; the module runs it directly
  from this repo, so edits deploy with the next switch.

## Host gotchas (grafton-router)

- No `python3`/`node`/`openssl`/`websocat` on PATH.
- Rust for local work: `nix shell nixpkgs#rustc nixpkgs#cargo nixpkgs#gcc`
  (there is no `nixpkgs#cc` flake attr — the flake package builds via
  `rustPlatform.buildRustPackage`).
- Format Nix with `nixfmt` (installed on the system).
