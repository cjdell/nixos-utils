use std::collections::HashMap;

use serde_json::Value;

use crate::podman::{ContainerInfo, StatsInfo, UpdateResult};
use crate::state::Config;

pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

const CSS: &str = r#"
:root { color-scheme: dark; }
* { box-sizing: border-box; }
body { font-family: system-ui, -apple-system, sans-serif; margin: 0; background: #14161b; color: #d7dae0; }
a { color: #7ab8f5; text-decoration: none; }
a:hover { text-decoration: underline; }
header { display: flex; align-items: baseline; gap: 1rem; padding: 0.7rem 1.25rem; border-bottom: 1px solid #262b34; background: #191c22; position: sticky; top: 0; z-index: 1; }
header h1 { font-size: 1.05rem; margin: 0; font-weight: 600; }
header .spacer { flex: 1; }
main { padding: 1.25rem; max-width: 1100px; margin: 0 auto; }
h2 { font-size: 1.3rem; margin-top: 0; }
h3 { font-size: 0.95rem; margin-top: 1.5rem; color: #aeb4bf; }
table { border-collapse: collapse; width: 100%; }
th, td { text-align: left; padding: 0.4rem 0.6rem; border-bottom: 1px solid #22262e; vertical-align: top; }
th { color: #8b919d; font-weight: 600; font-size: 0.72rem; text-transform: uppercase; letter-spacing: 0.04em; }
tbody tr:hover td { background: #181b21; }
.state-running { color: #4ade80; white-space: nowrap; }
.state-stopped { color: #f87171; white-space: nowrap; }
.dim { color: #767d89; }
.mono { font-family: ui-monospace, SFMono-Regular, Menlo, monospace; font-size: 0.85em; word-break: break-all; }
button { background: #262c39; color: #d7dae0; border: 1px solid #39414f; border-radius: 6px; padding: 0.32rem 0.85rem; cursor: pointer; font-size: 0.88rem; }
button:hover { background: #2f3745; }
button.warn { background: #4a2d2d; border-color: #6b3f3f; }
button.warn:hover { background: #5a3636; }
form.inline { display: inline; }
.banner { padding: 0.55rem 0.9rem; border-radius: 6px; margin-bottom: 1rem; font-size: 0.92rem; }
.banner.ok { background: #14301e; border: 1px solid #1f5c34; }
.banner.err { background: #38191b; border: 1px solid #5c2528; }
pre#logs { background: #0e1014; border: 1px solid #22262e; border-radius: 8px; padding: 0.8rem; height: 62vh; overflow-y: auto; font-size: 0.78rem; line-height: 1.5; white-space: pre-wrap; word-break: break-word; margin: 0.5rem 0; }
pre.small { background: #0e1014; border: 1px solid #22262e; border-radius: 8px; padding: 0.7rem; font-size: 0.78rem; line-height: 1.5; white-space: pre-wrap; word-break: break-all; margin: 0.25rem 0 0; }
dl.detail { display: grid; grid-template-columns: 150px 1fr; gap: 0.35rem 1rem; margin: 1rem 0; }
dl.detail dt { color: #8b919d; font-size: 0.82rem; padding-top: 0.1rem; }
dl.detail dd { margin: 0; }
ul.plain { list-style: none; padding: 0; margin: 0.25rem 0; }
#stream-status { font-size: 0.85rem; margin: 0.5rem 0; }
.actions { margin: 1.25rem 0; display: flex; gap: 0.75rem; align-items: center; flex-wrap: wrap; }
input[type="checkbox"] { width: 1.05rem; height: 1.05rem; vertical-align: middle; }
"#;

pub fn page(
    config: &Config,
    title: &str,
    username: &str,
    refresh: bool,
    body: String,
) -> String {
    let refresh_tag = if refresh {
        "<meta http-equiv=\"refresh\" content=\"30\">"
    } else {
        ""
    };
    let user_nav = if config.auth_enabled() {
        format!("<a href=\"/logout\">{} (logout)</a>", esc(username))
    } else {
        format!("<span class=\"dim\">{} (auth disabled)</span>", esc(username))
    };
    [
        "<!doctype html>",
        "<html lang=\"en\">",
        "<head>",
        "<meta charset=\"utf-8\">",
        "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">",
        refresh_tag,
        &format!("<title>{title} · Container UI</title>"),
        "<style>",
        CSS,
        "</style>",
        "</head>",
        "<body>",
        "<header>",
        "<h1><a href=\"/\">Containers</a></h1>",
        "<span class=\"spacer\"></span>",
        &user_nav,
        "</header>",
        "<main>",
        &body,
        "</main>",
        "</body>",
        "</html>",
    ]
    .join("\n")
}

pub fn error_page(config: &Config, username: &str, title: &str, message: &str) -> String {
    let body = [
        &format!("<h2>{}</h2>", esc(title)),
        &format!("<div class=\"banner err\">{}</div>", esc(message)),
        "<p><a href=\"/\">← back to containers</a></p>",
    ]
    .join("\n");
    page(config, title, username, false, body)
}

pub fn list_page(
    config: &Config,
    username: &str,
    csrf: &str,
    containers: &[ContainerInfo],
    stats: &HashMap<String, StatsInfo>,
) -> String {
    let mut rows = String::new();
    for c in containers {
        let st = stats.get(&c.name);
        let (state_class, dot) = if c.state == "running" {
            ("state-running", "●")
        } else {
            ("state-stopped", "○")
        };
        let ports = if c.ports.is_empty() {
            "<span class=\"dim\">—</span>".to_string()
        } else {
            c.ports
                .iter()
                .map(|p| format!("<span class=\"mono\">{}</span>", esc(p)))
                .collect::<Vec<_>>()
                .join("<br>")
        };
        let cpu = st
            .map(|s| esc(&s.cpu_percent))
            .unwrap_or_else(|| "—".to_string());
        let mem = st
            .map(|s| esc(&s.mem_usage))
            .unwrap_or_else(|| "—".to_string());
        rows.push_str(&format!(
            "<tr>\n\
             <td><input type=\"checkbox\" name=\"selected\" value=\"{name}\"></td>\n\
             <td><a href=\"/containers/{name}\">{name}</a></td>\n\
             <td class=\"{state_class}\">{dot} {state} <span class=\"dim\">({status})</span></td>\n\
             <td class=\"mono\">{image}</td>\n\
             <td>{ports}</td>\n\
             <td>{cpu}</td>\n\
             <td>{mem}</td>\n\
             <td><a href=\"/containers/{name}/logs\">logs</a></td>\n\
             </tr>\n",
            name = esc(&c.name),
            state = esc(&c.state),
            status = esc(&c.status),
            image = esc(&c.image),
        ));
    }
    let body = [
        "<p class=\"dim\">Refreshes every 30 s. Tick containers and press <em>Update selected</em> \
         to pull their images and restart the ones that changed.</p>",
        "<form method=\"post\" action=\"/update\">",
        &format!("<input type=\"hidden\" name=\"csrf\" value=\"{}\">", esc(csrf)),
        "<table>",
        "<thead><tr><th></th><th>Name</th><th>State</th><th>Image</th><th>Ports</th>\
         <th>CPU</th><th>Memory</th><th></th></tr></thead>",
        &format!("<tbody>{rows}</tbody>"),
        "</table>",
        "<p><button>Update selected</button></p>",
        "</form>",
        "<form method=\"post\" action=\"/update\" class=\"inline\">",
        &format!("<input type=\"hidden\" name=\"csrf\" value=\"{}\">", esc(csrf)),
        "<input type=\"hidden\" name=\"all\" value=\"1\">",
        "<button class=\"warn\">Update all</button>",
        "</form>",
    ]
    .join("\n");
    page(config, "containers", username, true, body)
}

pub fn container_page(
    config: &Config,
    username: &str,
    csrf: &str,
    name: &str,
    inspect: &Value,
    stats: Option<&StatsInfo>,
    msg: Option<&str>,
    error: Option<&str>,
) -> String {
    let state = inspect
        .get("State")
        .and_then(|s| s.get("Status"))
        .and_then(|s| s.as_str())
        .unwrap_or("?");
    let state_class = if state == "running" {
        "state-running"
    } else {
        "state-stopped"
    };
    let dot = if state == "running" { "●" } else { "○" };
    let image_name = inspect
        .get("ImageName")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let image_digest = inspect
        .get("ImageDigest")
        .and_then(|s| s.as_str())
        .unwrap_or("")
        .to_string();
    let net = inspect.get("NetworkSettings");
    let ip = net
        .and_then(|n| n.get("IPAddress"))
        .and_then(|s| s.as_str())
        .unwrap_or("");
    let gateway = net
        .and_then(|n| n.get("Gateway"))
        .and_then(|s| s.as_str())
        .unwrap_or("");
    let started = inspect
        .get("State")
        .and_then(|s| s.get("StartedAt"))
        .and_then(|s| s.as_str())
        .unwrap_or("");
    let exit_code = inspect
        .get("State")
        .and_then(|s| s.get("ExitCode"))
        .and_then(|s| s.as_i64());
    let oom = inspect
        .get("State")
        .and_then(|s| s.get("OOMKilled"))
        .and_then(|s| s.as_bool())
        .unwrap_or(false);
    let created = inspect
        .get("Created")
        .and_then(|s| s.as_str())
        .unwrap_or("");

    let mut ports: Vec<String> = Vec::new();
    if let Some(ports_map) = net
        .and_then(|n| n.get("Ports"))
        .and_then(|p| p.as_object())
    {
        let mut keys: Vec<&String> = ports_map.keys().collect();
        keys.sort();
        for k in keys {
            let bindings = ports_map
                .get(k)
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            if bindings.is_empty() {
                ports.push(format!("{k} (exposed)"));
            } else {
                for b in &bindings {
                    let host_ip = b.get("HostIp").and_then(|s| s.as_str()).unwrap_or("");
                    let host_port = b.get("HostPort").and_then(|s| s.as_str()).unwrap_or("");
                    ports.push(format!("{host_ip}:{host_port} → {k}"));
                }
            }
        }
    }

    let mut mounts_rows = String::new();
    if let Some(mounts) = inspect.get("Mounts").and_then(|m| m.as_array()) {
        for m in mounts {
            mounts_rows.push_str(&format!(
                "<tr><td class=\"mono\">{src}</td><td class=\"mono\">{dst}</td>\
                 <td>{ty}</td><td class=\"mono\">{mode}</td><td>{rw}</td></tr>",
                src = esc(m.get("Source").and_then(|s| s.as_str()).unwrap_or("")),
                dst = esc(m.get("Destination").and_then(|s| s.as_str()).unwrap_or("")),
                ty = esc(m.get("Type").and_then(|s| s.as_str()).unwrap_or("")),
                mode = esc(m.get("Mode").and_then(|s| s.as_str()).unwrap_or("")),
                rw = if m.get("RW").and_then(|s| s.as_bool()).unwrap_or(false) {
                    "rw"
                } else {
                    "ro"
                },
            ));
        }
    }

    let env: String = inspect
        .get("Config")
        .and_then(|c| c.get("Env"))
        .and_then(|e| e.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str())
                .map(esc)
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    let labels: String = inspect
        .get("Config")
        .and_then(|c| c.get("Labels"))
        .and_then(|l| l.as_object())
        .map(|obj| {
            let mut pairs: Vec<(String, String)> = obj
                .iter()
                .map(|(k, v)| (k.clone(), v.as_str().unwrap_or_default().to_string()))
                .collect();
            pairs.sort();
            pairs
                .into_iter()
                .map(|(k, v)| format!("{k} = {v}"))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default();

    let unit = inspect
        .get("Config")
        .and_then(|c| c.get("Labels"))
        .and_then(|l| l.get("PODMAN_SYSTEMD_UNIT"))
        .and_then(|u| u.as_str())
        .unwrap_or("");

    let banner = match (error, msg) {
        (Some(e), _) => format!("<div class=\"banner err\">{}</div>", esc(e)),
        (None, Some(m)) => format!("<div class=\"banner ok\">{}</div>", esc(m)),
        _ => String::new(),
    };

    let stats_row = match stats {
        Some(s) => format!(
            "<tr><td>{}</td><td>{}</td></tr>",
            esc(&s.cpu_percent),
            esc(&s.mem_usage)
        ),
        None => "<tr><td class=\"dim\">—</td><td class=\"dim\">—</td></tr>".to_string(),
    };

    let ports_html = if ports.is_empty() {
        "<span class=\"dim\">none</span>".to_string()
    } else {
        format!(
            "<ul class=\"plain mono\">{}</ul>",
            ports
                .iter()
                .map(|p| format!("<li>{}</li>", esc(p)))
                .collect::<Vec<_>>()
                .join("")
        )
    };

    let name_e = esc(name);
    let csrf_e = esc(csrf);
    let body = [
        &format!(
            "<h2>{name_e} <span class=\"{state_class}\">{dot} {state}</span></h2>"
        ),
        &banner,
        "<dl class=\"detail\">",
        &format!("<dt>Image</dt><dd class=\"mono\">{}</dd>", esc(&image_name)),
        &format!("<dt>Image digest</dt><dd class=\"mono\">{}</dd>", esc(&image_digest)),
        &format!(
            "<dt>Network</dt><dd class=\"mono\">{} <span class=\"dim\">(gw {})</span></dd>",
            esc(ip),
            esc(gateway)
        ),
        &format!("<dt>Created</dt><dd>{}</dd>", esc(created)),
        &format!("<dt>Started</dt><dd>{}</dd>", esc(started)),
        &format!(
            "<dt>Exit code</dt><dd>{}</dd>",
            exit_code.map(|c| c.to_string()).unwrap_or_else(|| "—".into())
        ),
        &format!("<dt>OOM killed</dt><dd>{}</dd>", if oom { "yes" } else { "no" }),
        &format!("<dt>Systemd unit</dt><dd class=\"mono\">{}</dd>", esc(unit)),
        "</dl>",
        "<h3>Ports</h3>",
        &ports_html,
        "<h3>Mounts</h3>",
        &format!(
            "<table><thead><tr><th>Host</th><th>Container</th><th>Type</th><th>Mode</th><th>Access</th></tr></thead>\
             <tbody>{}</tbody></table>",
            mounts_rows
        ),
        "<h3>Usage</h3>",
        &format!(
            "<table><thead><tr><th>CPU</th><th>Memory</th></tr></thead><tbody>{}</tbody></table>",
            stats_row
        ),
        "<h3>Environment</h3>",
        &format!("<pre class=\"small\">{}</pre>", if env.is_empty() { "—".into() } else { env }),
        "<h3>Labels</h3>",
        &format!("<pre class=\"small\">{}</pre>", if labels.is_empty() { "—".into() } else { labels }),
        "<div class=\"actions\">",
        &format!("<a href=\"/containers/{name_e}/logs\">View logs →</a>"),
        &format!(
            "<form method=\"post\" action=\"/containers/{name_e}/restart\" class=\"inline\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{csrf_e}\">\
             <button class=\"warn\">Restart</button></form>"
        ),
        &format!(
            "<form method=\"post\" action=\"/update\" class=\"inline\">\
             <input type=\"hidden\" name=\"csrf\" value=\"{csrf_e}\">\
             <input type=\"hidden\" name=\"selected\" value=\"{name_e}\">\
             <button>Update image</button></form>"
        ),
        "</div>",
    ]
    .join("\n");
    page(config, &esc(name), username, true, body)
}

pub fn logs_page(
    config: &Config,
    username: &str,
    name: &str,
    lines: &[String],
    last_ts: Option<&str>,
) -> String {
    let history: String = lines
        .iter()
        .map(|l| format!("{}\n", esc(l)))
        .collect();
    let since_js = match last_ts {
        Some(ts) => serde_json::to_string(ts).unwrap(),
        None => "null".to_string(),
    };
    let last_line_js = match lines.last() {
        Some(l) => serde_json::to_string(l).unwrap(),
        None => "null".to_string(),
    };
    let name_js = serde_json::to_string(name).unwrap();
    let script = format!(
        r#"<script>
(function () {{
  var pre = document.getElementById('logs');
  var status = document.getElementById('stream-status');
  var since = {since_js};
  var lastLine = {last_line_js};
  var MAX_LINES = 5000;
  function connect() {{
    var url = '/containers/' + {name_js} + '/logs/stream' +
      (since ? '?since=' + encodeURIComponent(since).replace(/\+/g, '%2B') : '');
    var es = new EventSource(url);
    es.onopen = function () {{
      status.textContent = 'live';
      status.className = 'state-running';
    }};
    es.onmessage = function (e) {{
      var line = e.data;
      if (line === lastLine) {{ lastLine = null; return; }}
      lastLine = line;
      var atBottom = pre.scrollTop + pre.clientHeight >= pre.scrollHeight - 30;
      pre.textContent += line + '\n';
      var all = pre.textContent.split('\n');
      if (all.length > MAX_LINES) {{
        pre.textContent = all.slice(-MAX_LINES).join('\n');
      }}
      if (atBottom) pre.scrollTop = pre.scrollHeight;
    }};
    es.onerror = function () {{
      status.textContent = 'disconnected — retrying in 2 s';
      status.className = 'state-stopped';
      es.close();
      setTimeout(connect, 2000);
    }};
  }}
  // Start at the latest line (tail -f style) so no manual scroll is needed
  pre.scrollTop = pre.scrollHeight;
  connect();
}})();
</script>"#,
        since_js = since_js,
        last_line_js = last_line_js,
        name_js = name_js,
    );
    let name_e = esc(name);
    let body = [
        &format!("<p><a href=\"/containers/{name_e}\">← {name_e}</a></p>"),
        "<div id=\"stream-status\" class=\"dim\">connecting…</div>",
        &format!("<pre id=\"logs\">{history}</pre>"),
        &script,
    ]
    .join("\n");
    page(config, &format!("logs · {}", esc(name)), username, false, body)
}

pub fn update_results_page(
    config: &Config,
    username: &str,
    results: &[UpdateResult],
) -> String {
    let mut rows = String::new();
    if results.is_empty() {
        rows.push_str("<tr><td colspan=\"5\" class=\"dim\">No containers selected.</td></tr>");
    }
    for r in results {
        let (status_cell, cls) = if let Some(e) = &r.error {
            (format!("error: {e}"), "state-stopped")
        } else if r.changed {
            ("updated — restarted".to_string(), "state-running")
        } else {
            ("unchanged".to_string(), "dim")
        };
        rows.push_str(&format!(
            "<tr><td><a href=\"/containers/{name}\">{name}</a></td>\
             <td class=\"mono\">{image}</td>\
             <td class=\"mono\">{old}</td>\
             <td class=\"mono\">{new}</td>\
             <td class=\"{cls}\">{status}</td></tr>",
            name = esc(&r.name),
            image = esc(&r.image),
            old = r.old.as_deref().map(esc).unwrap_or_else(|| "—".into()),
            new = r.new.as_deref().map(esc).unwrap_or_else(|| "—".into()),
            status = esc(&status_cell),
        ));
    }
    let body = [
        "<h2>Update results</h2>",
        "<table>",
        "<thead><tr><th>Container</th><th>Image</th><th>Old digest</th>\
         <th>New digest</th><th>Status</th></tr></thead>",
        &format!("<tbody>{rows}</tbody>"),
        "</table>",
        "<p><a href=\"/\">← back to containers</a></p>",
    ]
    .join("\n");
    page(config, "update results", username, false, body)
}
