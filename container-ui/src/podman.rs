use std::collections::HashMap;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdout, Command};

use crate::state::Config;

#[derive(Clone, Debug)]
pub struct ContainerInfo {
    pub name: String,
    pub image: String,
    pub state: String,
    pub status: String,
    pub ports: Vec<String>,
    pub unit: String,
}

#[derive(Clone, Debug, Default)]
pub struct StatsInfo {
    pub cpu_percent: String,
    pub mem_usage: String,
    pub mem_percent: String,
}

#[derive(Clone, Debug)]
pub struct UpdateResult {
    pub name: String,
    pub image: String,
    pub old: Option<String>,
    pub new: Option<String>,
    pub changed: bool,
    pub error: Option<String>,
}

#[derive(Clone)]
pub struct Podman {
    bin: String,
    systemctl: String,
    timeout: Duration,
}

impl Podman {
    pub fn new(config: &Config) -> Self {
        Podman {
            bin: config.podman_bin.clone(),
            systemctl: config.systemctl_bin.clone(),
            timeout: Duration::from_secs(600),
        }
    }

    fn podman(&self, args: &[&str]) -> Command {
        let mut c = Command::new(&self.bin);
        c.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
        c
    }

    fn systemctl(&self, args: &[&str]) -> Command {
        let mut c = Command::new(&self.systemctl);
        c.args(args).stdout(Stdio::piped()).stderr(Stdio::piped());
        c
    }

    async fn run_json(&self, args: &[&str]) -> Result<serde_json::Value, String> {
        let output = self
            .podman(args)
            .output()
            .await
            .map_err(|e| format!("spawn podman: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "podman {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
        serde_json::from_slice(&output.stdout).map_err(|e| format!("invalid JSON from podman: {e}"))
    }

    pub async fn list(&self) -> Result<Vec<ContainerInfo>, String> {
        let v = self.run_json(&["ps", "-a", "--format=json"]).await?;
        let mut out = Vec::new();
        for item in v.as_array().into_iter().flatten() {
            out.push(ContainerInfo {
                name: item
                    .get("Names")
                    .and_then(|n| n.as_array())
                    .and_then(|a| a.first())
                    .and_then(|n| n.as_str())
                    .unwrap_or_default()
                    .trim_start_matches('/')
                    .to_string(),
                image: item
                    .get("Image")
                    .and_then(|i| i.as_str())
                    .unwrap_or_default()
                    .to_string(),
                state: item
                    .get("State")
                    .and_then(|s| s.as_str())
                    .unwrap_or_default()
                    .to_string(),
                status: item
                    .get("Status")
                    .and_then(|s| s.as_str())
                    .unwrap_or_default()
                    .to_string(),
                ports: item
                    .get("Ports")
                    .and_then(|p| p.as_array())
                    .map(|ps| ps.iter().map(format_port).collect())
                    .unwrap_or_default(),
                unit: item
                    .get("Labels")
                    .and_then(|l| l.get("PODMAN_SYSTEMD_UNIT"))
                    .and_then(|u| u.as_str())
                    .unwrap_or_default()
                    .to_string(),
            });
        }
        Ok(out)
    }

    pub async fn stats(&self) -> Result<HashMap<String, StatsInfo>, String> {
        let v = self
            .run_json(&["stats", "--all", "--no-stream", "--format=json"])
            .await?;
        let mut map = HashMap::new();
        for item in v.as_array().into_iter().flatten() {
            map.insert(
                item.get("name")
                    .and_then(|n| n.as_str())
                    .unwrap_or_default()
                    .to_string(),
                parse_stats(item),
            );
        }
        Ok(map)
    }

    pub async fn stats_one(&self, name: &str) -> Result<Option<StatsInfo>, String> {
        let v = self
            .run_json(&[
                "stats",
                "--no-stream",
                "--format=json",
                name,
            ])
            .await?;
        Ok(v.as_array()
            .and_then(|a| a.first())
            .map(parse_stats))
    }

    pub async fn inspect(&self, name: &str) -> Result<serde_json::Value, String> {
        let v = self.run_json(&["inspect", name, "--format=json"]).await?;
        v.as_array()
            .and_then(|a| a.first())
            .cloned()
            .ok_or_else(|| "empty inspect result".to_string())
    }

    pub async fn image_digest(&self, image: &str) -> Result<String, String> {
        let v = self.run_json(&["inspect", image]).await?;
        v.as_array()
            .and_then(|a| a.first())
            .and_then(|i| i.get("Digest"))
            .and_then(|d| d.as_str())
            .map(short_digest)
            .ok_or_else(|| "image has no digest".to_string())
    }

    pub async fn pull(&self, image: &str) -> Result<(), String> {
        let mut cmd = self.podman(&["pull", image]);
        cmd.kill_on_drop(true);
        let output = tokio::time::timeout(self.timeout, cmd.output())
            .await
            .map_err(|_| "podman pull timed out".to_string())?
            .map_err(|e| format!("spawn podman pull: {e}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        Ok(())
    }

    pub async fn restart_unit(&self, unit: &str) -> Result<(), String> {
        let mut cmd = self.systemctl(&["restart", unit]);
        cmd.kill_on_drop(true);
        let output = tokio::time::timeout(self.timeout, cmd.output())
            .await
            .map_err(|_| "systemctl restart timed out".to_string())?
            .map_err(|e| format!("spawn systemctl: {e}"))?;
        if !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        Ok(())
    }

    pub async fn logs_tail(&self, name: &str, tail: u64) -> Result<Vec<String>, String> {
        let tail_arg = tail.to_string();
        let mut cmd = self.podman(&["logs", "--timestamps", "--tail", &tail_arg, name]);
        cmd.kill_on_drop(true);
        let output = tokio::time::timeout(self.timeout, cmd.output())
            .await
            .map_err(|_| "podman logs timed out".to_string())?
            .map_err(|e| format!("spawn podman logs: {e}"))?;
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        if stdout.trim().is_empty() && !output.status.success() {
            return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
        }
        Ok(stdout.lines().map(|l| l.to_string()).collect())
    }

    /// Follow logs of a container as a stream of lines. `since` is an
    /// RFC3339 timestamp (inclusive); the podman process is killed if the
    /// stream is dropped early (e.g. client disconnect).
    pub fn logs_follow(
        &self,
        name: &str,
        since: Option<&str>,
    ) -> impl futures::Stream<Item = Result<String, String>> + Send {
        logs_follow_stream(&self.bin, name, since)
    }
}

fn parse_stats(item: &serde_json::Value) -> StatsInfo {
    StatsInfo {
        cpu_percent: item
            .get("cpu_percent")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string(),
        mem_usage: item
            .get("mem_usage")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string(),
        mem_percent: item
            .get("mem_percent")
            .and_then(|s| s.as_str())
            .unwrap_or_default()
            .to_string(),
    }
}

fn format_port(p: &serde_json::Value) -> String {
    let cport = p.get("container_port").and_then(|v| v.as_u64()).unwrap_or(0);
    let hport = p.get("host_port").and_then(|v| v.as_u64()).unwrap_or(0);
    let proto = p.get("protocol").and_then(|s| s.as_str()).unwrap_or("tcp");
    if hport > 0 {
        format!("{hport}->{cport}/{proto}")
    } else {
        format!("{cport}/{proto}")
    }
}

pub fn short_digest(digest: &str) -> String {
    digest
        .strip_prefix("sha256:")
        .unwrap_or(digest)
        .get(..12)
        .unwrap_or(digest)
        .to_string()
}

/// A child process that is killed when dropped (covers client disconnects).
struct TrackedChild(Child);

impl TrackedChild {
    fn take_stdout(&mut self) -> Option<ChildStdout> {
        self.0.stdout.take()
    }

    fn take_stderr(&mut self) -> Option<ChildStderr> {
        self.0.stderr.take()
    }

    async fn wait(&mut self) {
        let _ = self.0.wait().await;
    }
}

impl Drop for TrackedChild {
    fn drop(&mut self) {
        let _ = self.0.start_kill();
    }
}

struct FollowConfig {
    bin: String,
    name: String,
    since: Option<String>,
}

struct FollowState {
    lines: tokio::io::Lines<BufReader<ChildStdout>>,
    child: TrackedChild,
}

enum FollowPhase {
    Unstarted,
    Running(FollowState),
    Done,
}

pub fn logs_follow_stream(
    bin: &str,
    name: &str,
    since: Option<&str>,
) -> impl futures::Stream<Item = Result<String, String>> + Send {
    let config = std::sync::Arc::new(FollowConfig {
        bin: bin.to_string(),
        name: name.to_string(),
        since: since.map(str::to_string),
    });

    futures::stream::unfold(FollowPhase::Unstarted, move |phase| {
        let config = config.clone();
        async move {
            let mut running = match phase {
                FollowPhase::Unstarted => match spawn_follow(&config).await {
                    Ok(r) => r,
                    Err(e) => return Some((Err(e), FollowPhase::Done)),
                },
                FollowPhase::Running(r) => r,
                FollowPhase::Done => return None,
            };
            match running.lines.next_line().await {
                Ok(Some(line)) => Some((Ok(line), FollowPhase::Running(running))),
                Ok(None) => {
                    running.child.wait().await;
                    None
                }
                Err(e) => Some((
                    Err(format!("log stream read error: {e}")),
                    FollowPhase::Running(running),
                )),
            }
        }
    })
}

async fn spawn_follow(config: &FollowConfig) -> Result<FollowState, String> {
    let mut cmd = Command::new(&config.bin);
    cmd.arg("logs").arg("--timestamps").arg("--follow");
    if let Some(s) = &config.since {
        cmd.arg("--since").arg(s);
    }
    cmd.arg(&config.name);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let child = cmd.spawn().map_err(|e| format!("spawn podman logs: {e}"))?;
    let mut tracked = TrackedChild(child);
    if let Some(mut stderr) = tracked.take_stderr() {
        tokio::spawn(async move {
            let mut buf = Vec::new();
            let _ = stderr.read_to_end(&mut buf).await;
            tracing::debug!("podman logs stderr: {}", String::from_utf8_lossy(&buf));
        });
    }
    let stdout = tracked.take_stdout().expect("stdout is piped");
    Ok(FollowState {
        lines: BufReader::new(stdout).lines(),
        child: tracked,
    })
}

pub async fn send_webhook(http: &reqwest::Client, url: &str, title: &str, message: &str) {
    let body = serde_json::json!({ "title": title, "message": message });
    match http.post(url).json(&body).send().await {
        Ok(resp) => {
            let status = resp.status();
            if status.is_success() {
                tracing::info!(%status, "webhook sent");
            } else {
                tracing::warn!(%status, "webhook rejected");
            }
        }
        Err(e) => tracing::warn!(error = %e, "webhook send failed"),
    }
}
