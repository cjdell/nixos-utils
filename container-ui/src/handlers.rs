use axum::body::Bytes;
use axum::extract::{Form, Path, Query, Request, State};
use axum::http::{header, HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{sse::Event, sse::KeepAlive, Html, IntoResponse, Redirect, Response, Sse};
use axum::routing::{get, post};
use axum::Router;
use futures::StreamExt;
use serde::Deserialize;

use crate::html;
use crate::oidc;
use crate::podman::{self, ContainerInfo, UpdateResult};
use crate::state::{
    AppState, PendingAuth, Session, SESSION_COOKIE, cookie_value, random_hex,
};

#[derive(Clone)]
pub struct Authed {
    pub username: String,
    pub csrf: String,
}

impl axum::extract::FromRequestParts<AppState> for Authed {
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut axum::http::request::Parts,
        _state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<Authed>()
            .cloned()
            .ok_or((StatusCode::UNAUTHORIZED, "not authenticated"))
    }
}

pub fn router(state: AppState) -> Router {
    let auth = axum::middleware::from_fn_with_state(state.clone(), auth_middleware);
    Router::new()
        .route("/", get(list_page))
        .route("/containers/{name}", get(container_page))
        .route("/containers/{name}/logs", get(logs_page))
        .route("/containers/{name}/logs/stream", get(logs_stream))
        .route("/containers/{name}/restart", post(restart_handler))
        .route("/update", post(update_handler))
        .route("/login", get(login_handler))
        .route("/oidc/callback", get(callback_handler))
        .route("/logout", get(logout_handler))
        .route("/health", get(|| async { "ok" }))
        .layer(auth)
        .layer(tower_http::trace::TraceLayer::new_for_http())
        .with_state(state)
}

async fn auth_middleware(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Response {
    if !state.config.auth_enabled() {
        let mut request = request;
        request.extensions_mut().insert(Authed {
            username: "dev".to_string(),
            csrf: String::new(),
        });
        return next.run(request).await;
    }
    let path = request.uri().path().to_string();
    let is_public =
        path == "/health" || path == "/login" || path == "/oidc/callback";
    if is_public {
        return next.run(request).await;
    }
    let token = cookie_value(&headers, SESSION_COOKIE);
    let session = if let Some(t) = token {
        state.sessions.lock().await.get(&t).cloned()
    } else {
        None
    };
    let Some(session) = session else {
        return unauthorized(request);
    };
    let mut request = request;
    request.extensions_mut().insert(Authed {
        username: session.username,
        csrf: session.csrf,
    });
    next.run(request).await
}

fn unauthorized(request: Request) -> Response {
    if request.method() == Method::GET {
        Redirect::to("/login").into_response()
    } else {
        (StatusCode::UNAUTHORIZED, "authentication required").into_response()
    }
}

fn html_resp(status: StatusCode, body: String) -> Response {
    (status, Html(body)).into_response()
}

fn check_csrf(authed: &Authed, provided: &str) -> Result<(), (StatusCode, String)> {
    if authed.csrf.is_empty() {
        // dev mode (OIDC not configured)
        return Ok(());
    }
    if provided == authed.csrf {
        Ok(())
    } else {
        Err((
            StatusCode::FORBIDDEN,
            "invalid or missing CSRF token".to_string(),
        ))
    }
}

async fn list_page(
    State(state): State<AppState>,
    authed: Authed,
) -> impl IntoResponse {
    let (list, stats) = tokio::join!(state.podman.list(), state.podman.stats());
    let containers = match list {
        Ok(c) => c,
        Err(e) => {
            return html_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                html::error_page(&state.config, &authed.username, "containers", &e),
            )
        }
    };
    let stats = stats.unwrap_or_default();
    html_resp(
        StatusCode::OK,
        html::list_page(
            &state.config,
            &authed.username,
            &authed.csrf,
            &containers,
            &stats,
        ),
    )
}

#[derive(Deserialize)]
struct DetailQuery {
    #[serde(default)]
    msg: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

async fn container_page(
    State(state): State<AppState>,
    authed: Authed,
    Path(name): Path<String>,
    Query(q): Query<DetailQuery>,
) -> impl IntoResponse {
    let inspect = match state.podman.inspect(&name).await {
        Ok(v) => v,
        Err(e) => {
            return html_resp(
                StatusCode::NOT_FOUND,
                html::error_page(&state.config, &authed.username, &name, &e),
            )
        }
    };
    let stats = state.podman.stats_one(&name).await.ok().flatten();
    html_resp(
        StatusCode::OK,
        html::container_page(
            &state.config,
            &authed.username,
            &authed.csrf,
            &name,
            &inspect,
            stats.as_ref(),
            q.msg.as_deref(),
            q.error.as_deref(),
        ),
    )
}

async fn logs_page(
    State(state): State<AppState>,
    authed: Authed,
    Path(name): Path<String>,
) -> impl IntoResponse {
    let lines = match state.podman.logs_tail(&name, 500).await {
        Ok(l) => l,
        Err(e) => {
            return html_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                html::error_page(&state.config, &authed.username, &name, &e),
            )
        }
    };
    let last_ts = lines
        .last()
        .and_then(|l| l.split_whitespace().next())
        .map(str::to_string);
    html_resp(
        StatusCode::OK,
        html::logs_page(
            &state.config,
            &authed.username,
            &name,
            &lines,
            last_ts.as_deref(),
        ),
    )
}

#[derive(Deserialize)]
struct StreamQuery {
    since: Option<String>,
}

async fn logs_stream(
    State(state): State<AppState>,
    _authed: Authed,
    Path(name): Path<String>,
    Query(q): Query<StreamQuery>,
) -> Sse<impl futures::Stream<Item = Result<Event, axum::Error>> + Send> {
    let stream = state.podman.logs_follow(&name, q.since.as_deref());
    let stream = stream.map(|item| match item {
        Ok(line) => Ok(Event::default().data(line)),
        Err(e) => Err(axum::Error::new(anyhow::anyhow!("{e}"))),
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn update_handler(
    State(state): State<AppState>,
    authed: Authed,
    body: Bytes,
) -> impl IntoResponse {
    // Parse the form body manually so repeated `selected` keys are preserved
    // (serde_urlencoded rejects duplicate keys for struct fields).
    let pairs: Vec<(String, String)> = url::form_urlencoded::parse(&body)
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .collect();
    let csrf = pairs
        .iter()
        .find(|(k, _)| k == "csrf")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let selected: Vec<String> = pairs
        .iter()
        .filter(|(k, _)| k == "selected")
        .map(|(_, v)| v.clone())
        .collect();
    let all = pairs.iter().any(|(k, _)| k == "all");

    if let Err(e) = check_csrf(&authed, &csrf) {
        return e.into_response();
    }
    let containers = match state.podman.list().await {
        Ok(c) => c,
        Err(e) => {
            return html_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                html::error_page(&state.config, &authed.username, "update", &e),
            )
        }
    };
    let targets: Vec<ContainerInfo> = if all {
        containers
    } else if selected.is_empty() {
        Vec::new()
    } else {
        containers
            .into_iter()
            .filter(|c| selected.iter().any(|s| *s == c.name))
            .collect()
    };

    let mut results: Vec<UpdateResult> = Vec::new();
    for c in &targets {
        results.push(run_update(&state, c).await);
    }

    let changed_msgs: Vec<String> = results
        .iter()
        .filter(|r| r.changed)
        .map(|r| {
            format!(
                "🔄 Updated \"{}\" - {} => {}",
                r.image,
                r.old.clone().unwrap_or_default(),
                r.new.clone().unwrap_or_default()
            )
        })
        .collect();
    if !changed_msgs.is_empty() {
        if let Some(url) = state.config.webhook_url.clone() {
            podman::send_webhook(&state.http, &url, "Updated Containers", &changed_msgs.join("\n"))
                .await;
        }
    }

    html_resp(
        StatusCode::OK,
        html::update_results_page(&state.config, &authed.username, &results),
    )
}

async fn run_update(state: &AppState, c: &ContainerInfo) -> UpdateResult {
    let mut res = UpdateResult {
        name: c.name.clone(),
        image: c.image.clone(),
        old: None,
        new: None,
        changed: false,
        error: None,
    };
    match state.podman.image_digest(&c.image).await {
        Ok(d) => res.old = Some(d),
        Err(e) => {
            res.error = Some(format!("get old digest: {e}"));
            return res;
        }
    }
    match state.podman.pull(&c.image).await {
        Ok(()) => {}
        Err(e) => {
            res.error = Some(format!("pull: {e}"));
            return res;
        }
    }
    match state.podman.image_digest(&c.image).await {
        Ok(d) => res.new = Some(d),
        Err(e) => {
            res.error = Some(format!("get new digest: {e}"));
            return res;
        }
    }
    if res.old != res.new {
        res.changed = true;
        let unit = unit_for(c);
        match state.podman.restart_unit(&unit).await {
            Ok(()) => {}
            Err(e) => res.error = Some(format!("image changed but restart failed: {e}")),
        }
    }
    res
}

fn unit_for(c: &ContainerInfo) -> String {
    if c.unit.is_empty() {
        format!("podman-{}.service", c.name)
    } else {
        c.unit.clone()
    }
}

#[derive(Deserialize)]
struct RestartForm {
    csrf: String,
}

async fn restart_handler(
    State(state): State<AppState>,
    authed: Authed,
    Path(name): Path<String>,
    Form(form): Form<RestartForm>,
) -> impl IntoResponse {
    if let Err(e) = check_csrf(&authed, &form.csrf) {
        return e.into_response();
    }
    let containers = match state.podman.list().await {
        Ok(c) => c,
        Err(e) => {
            return html_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                html::error_page(&state.config, &authed.username, &name, &e),
            )
        }
    };
    let Some(c) = containers.iter().find(|c| c.name == name) else {
        return html_resp(
            StatusCode::NOT_FOUND,
            html::error_page(&state.config, &authed.username, &name, "no such container"),
        );
    };
    let unit = unit_for(c);
    match state.podman.restart_unit(&unit).await {
        Ok(()) => {
            let url = format!(
                "/containers/{name}?msg={}",
                url::form_urlencoded::byte_serialize(unit.as_bytes()).collect::<String>()
            );
            Redirect::to(&url).into_response()
        }
        Err(e) => {
            let msg = format!("restart {unit} failed: {e}");
            let url = format!(
                "/containers/{name}?error={}",
                url::form_urlencoded::byte_serialize(msg.as_bytes()).collect::<String>()
            );
            Redirect::to(&url).into_response()
        }
    }
}

async fn login_handler(State(state): State<AppState>) -> impl IntoResponse {
    let Some(oidc) = &state.oidc else {
        return (StatusCode::SERVICE_UNAVAILABLE, "auth not enabled").into_response();
    };
    let disc = match oidc.discover(&state.http).await {
        Ok(d) => d,
        Err(e) => {
            return html_resp(
                StatusCode::INTERNAL_SERVER_ERROR,
                html::error_page(&state.config, "login", "login", &format!("OIDC discovery failed: {e}")),
            )
        }
    };
    let verifier = oidc::code_verifier();
    let challenge = oidc::code_challenge(&verifier);
    let state_val = state
        .insert_pending(PendingAuth {
            code_verifier: verifier,
            created_at: std::time::SystemTime::now(),
        })
        .await;
    let url = oidc.authorize_url(&disc, &state_val, &challenge);
    (
        StatusCode::SEE_OTHER,
        [(header::LOCATION, url)],
    )
        .into_response()
}

#[derive(Deserialize)]
struct CallbackQuery {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

async fn callback_handler(
    State(state): State<AppState>,
    Query(q): Query<CallbackQuery>,
) -> impl IntoResponse {
    let fail = |msg: String| {
        html_resp(
            StatusCode::UNAUTHORIZED,
            html::error_page(&state.config, "login", "login", &msg),
        )
    };

    let (code, state_val) = match (&q.code, &q.state) {
        (Some(c), Some(s)) => (c.clone(), s.clone()),
        _ => {
            let msg = q
                .error
                .clone()
                .unwrap_or_else(|| "missing code or state parameter".to_string());
            return fail(msg);
        }
    };
    let Some(pending) = state.pending.lock().await.remove(&state_val) else {
        return fail("unknown or expired state parameter".to_string());
    };
    let Some(oidc) = &state.oidc else {
        return fail("auth not enabled".to_string());
    };
    let tokens = match oidc.exchange_code(&state.http, &code, &pending.code_verifier).await {
        Ok(t) => t,
        Err(e) => return fail(e),
    };
    let Some(id_token) = tokens.id_token else {
        return fail("token response missing id_token".to_string());
    };
    let claims = match oidc.verify_id_token(&state.http, &id_token).await {
        Ok(c) => c,
        Err(e) => return fail(format!("id_token validation failed: {e}")),
    };
    let username = claims
        .preferred_username
        .or_else(|| claims.email.clone())
        .unwrap_or_else(|| claims.sub.clone());
    let session = Session {
        username,
        email: claims.email,
        csrf: random_hex(16),
        created_at: std::time::SystemTime::now(),
    };
    let token = state.insert_session(session).await;
    let secure = if state.config.secure_cookies() {
        "; Secure"
    } else {
        ""
    };
    let cookie = format!("{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax{secure}");
    (
        StatusCode::SEE_OTHER,
        [(
            header::LOCATION,
            "/".to_string(),
        ), (header::SET_COOKIE, cookie)],
    )
    .into_response()
}

async fn logout_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> impl IntoResponse {
    if let Some(token) = cookie_value(&headers, SESSION_COOKIE) {
        state.sessions.lock().await.remove(&token);
    }
    let cookie = format!("{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0");
    (
        StatusCode::SEE_OTHER,
        [
            (header::LOCATION, "/login".to_string()),
            (header::SET_COOKIE, cookie),
        ],
    )
    .into_response()
}
