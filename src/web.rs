use crate::{
    admin,
    config::Config,
    db::{self, Document, Recipient},
    links, mcp, pdf,
};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::{
    Router,
    extract::{ConnectInfo, DefaultBodyLimit, Form, Multipart, Path, Query, Request, State},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{Html, IntoResponse, Redirect, Response},
    routing::{get, post},
};
use serde::Deserialize;
use sqlx::{Row, SqlitePool};
use std::{net::SocketAddr, sync::Arc};

pub struct App {
    pub pool: SqlitePool,
    pub config: Config,
    pub link_key: [u8; 32],
}
type S = State<Arc<App>>;
type Result<T> = std::result::Result<T, Error>;
#[derive(Debug)]
pub struct Error(pub(crate) StatusCode, pub(crate) String);
impl IntoResponse for Error {
    fn into_response(self) -> Response {
        (self.0,Html(page("Noget gik galt",&format!("<section class='panel'><h1>Handlingen kunne ikke gennemføres</h1><p>{}</p><a href='/'>Til overblikket</a></section>",esc(&self.1)),None))).into_response()
    }
}
impl From<sqlx::Error> for Error {
    fn from(e: sqlx::Error) -> Self {
        eprintln!("Database operation failed: {e}");
        Self(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Der opstod en databasefejl. Prøv igen.".into(),
        )
    }
}
pub(crate) fn bad(s: &str) -> Error {
    Error(StatusCode::BAD_REQUEST, s.into())
}
pub(crate) fn denied() -> Error {
    Error(
        StatusCode::FORBIDDEN,
        "Adgang nægtet eller linket er ikke længere gyldigt.".into(),
    )
}
pub(crate) fn missing() -> Error {
    Error(StatusCode::NOT_FOUND, "Dokumentet findes ikke.".into())
}
pub fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}
fn cookie(h: &HeaderMap, name: &str) -> Option<String> {
    h.get(header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .filter_map(|s| s.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}
fn csrf_field(secret: &str) -> String {
    format!(
        "<input type='hidden' name='csrf' value='{}'>",
        db::csrf(secret)
    )
}
fn check_csrf(secret: &str, submitted: &str) -> Result<()> {
    if secret.len() != 64 || db::csrf(secret) != submitted {
        Err(denied())
    } else {
        Ok(())
    }
}
fn set_cookie(app: &App, name: &str, value: &str, seconds: i64) -> HeaderValue {
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={seconds}{}",
        if app.config.secure() { "; Secure" } else { "" }
    ))
    .unwrap()
}
fn page(title: &str, body: &str, session: Option<&str>) -> String {
    let nav = if let Some(secret) = session {
        format!(
            "<nav><a href='/'>Dokumenter</a><a href='/archive'>Signeret arkiv</a><a href='/integrations'>AI / MCP</a><a class='button small' href='/new'>Nyt dokument</a><form method='post' action='/logout'>{}<button class='quiet'>Log ud</button></form></nav>",
            csrf_field(secret)
        )
    } else {
        "<span class='nav-note'>Dokumenter. Aftaler. På plads.</span>".into()
    };
    format!(
        "<!doctype html><html lang='da'><head><meta charset='utf-8'><meta name='viewport' content='width=device-width, initial-scale=1'><title>{} · Skriv Under</title><link rel='icon' href='/favicon.ico'><link rel='icon' type='image/svg+xml' href='/favicon.svg' sizes='any'><link rel='apple-touch-icon' href='/apple-touch-icon.png'><script src='/assets/theme.js'></script><link rel='stylesheet' href='/assets/style.css'><script src='/assets/process.js' defer></script></head><body><a class='skip-link' href='#main'>Spring til indhold</a><header><a class='brand' href='/'><span class='brand-mark'>✓</span> Skriv Under</a><div class='header-actions'>{nav}<label class='theme-picker' for='theme'>Tema<select id='theme' name='theme'><option value='system'>System</option><option value='light'>Lyst</option><option value='dark'>Mørkt</option></select></label></div></header><main id='main'>{body}</main><footer>Skriv Under <span>Personlige links · Tydeligt samtykke · Dokumenteret forløb</span></footer></body></html>",
        esc(title)
    )
}
fn badge(status: &str) -> String {
    format!(
        "<span class='badge {}'>{}</span>",
        esc(status),
        match status {
            "draft" => "Kladde",
            "pending" => "Afventer",
            "completed" => "Gennemført",
            "signed" => "Underskrevet",
            "declined" => "Afvist",
            "expired" => "Udløbet",
            "cancelled" => "Annulleret",
            _ => status,
        }
    )
}
fn copy_link(id: &str, url: &str, label: &str) -> String {
    format!(
        "<div class='share-link'><label for='{}'>{}</label><div><input id='{}' type='text' readonly value='{}' autocomplete='off' spellcheck='false'><button type='button' class='secondary' data-copy-target='{}'>Kopiér link</button></div><small data-copy-note aria-live='polite'></small></div>",
        esc(id),
        esc(label),
        esc(id),
        esc(url),
        esc(id)
    )
}
async fn auth(app: &App, h: &HeaderMap) -> Result<(i64, String)> {
    let secret = cookie(h, "session").ok_or_else(denied)?;
    let user =
        sqlx::query_scalar("SELECT user_id FROM sessions WHERE token_hash=? AND expires_at>?")
            .bind(db::hash(&secret))
            .bind(db::now())
            .fetch_optional(&app.pool)
            .await?
            .ok_or_else(denied)?;
    Ok((user, secret))
}
pub(crate) async fn owned(app: &App, id: i64, user: i64) -> Result<Document> {
    sqlx::query_as("SELECT id,owner_id,title,message,status,original_hash,created_at,expires_at FROM documents WHERE id=? AND owner_id=?")
        .bind(id)
        .bind(user)
        .fetch_optional(&app.pool)
        .await?
        .ok_or_else(missing)
}
pub(crate) async fn recipients(app: &App, id: i64) -> Result<Vec<Recipient>> {
    Ok(
        sqlx::query_as("SELECT * FROM recipients WHERE document_id=? ORDER BY id")
            .bind(id)
            .fetch_all(&app.pool)
            .await?,
    )
}
async fn signer(app: &App, h: &HeaderMap) -> Result<(Recipient, Document, String)> {
    let secret = cookie(h, "signer").ok_or_else(denied)?;
    let r: Recipient = sqlx::query_as("SELECT * FROM recipients WHERE token_hash=?")
        .bind(db::hash(&secret))
        .fetch_optional(&app.pool)
        .await?
        .ok_or_else(denied)?;
    let d: Document = sqlx::query_as("SELECT id,owner_id,title,message,status,original_hash,created_at,expires_at FROM documents WHERE id=?")
        .bind(r.document_id)
        .fetch_one(&app.pool)
        .await?;
    Ok((r, d, secret))
}

pub fn router(app: Arc<App>) -> Router {
    Router::new()
        .route("/", get(dashboard))
        .route("/archive", get(archive))
        .route("/documents/{id}/status", get(process_status))
        .route("/integrations", get(integrations_page))
        .route("/integrations/tokens", post(create_mcp_token))
        .route("/integrations/tokens/{id}/revoke", post(revoke_mcp_token))
        .route(
            "/mcp",
            post(mcp::endpoint)
                .get(mcp::no_stream)
                .delete(mcp::no_stream)
                .layer(DefaultBodyLimit::max(30 * 1024 * 1024)),
        )
        .route(
            "/favicon.svg",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/svg+xml")],
                    include_str!("../assets/brand/favicon.svg"),
                )
            }),
        )
        .route(
            "/favicon.ico",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/x-icon")],
                    include_bytes!("../assets/brand/favicon.ico").as_slice(),
                )
            }),
        )
        .route(
            "/apple-touch-icon.png",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "image/png")],
                    include_bytes!("../assets/brand/apple-touch-icon.png").as_slice(),
                )
            }),
        )
        .route("/health", get(|| async { "ok" }))
        .route("/login", get(login_page).post(login))
        .route("/logout", post(logout))
        .route("/new", get(new_page))
        .route(
            "/documents",
            post(upload).layer(DefaultBodyLimit::max(22 * 1024 * 1024)),
        )
        .route("/documents/{id}", get(detail))
        .route("/documents/{id}/{action}", post(document_action))
        .route("/documents/{id}/files/{kind}", get(owner_file))
        .route("/documents/{id}/audit.json", get(audit_json))
        .route("/s/{token}", get(open_invitation))
        .route("/sign", get(sign_page).post(sign))
        .route("/sign/document.pdf", get(sign_file))
        .route("/sign/receipt.pdf", get(sign_receipt))
        .route("/d/{token}/{kind}", get(download))
        .route(
            "/assets/pdfjs/{kind}/{name}",
            get(|Path((kind, name)): Path<(String, String)>| async move {
                match crate::pdf_assets::asset(&kind, &name) {
                    Some(bytes) => (
                        [(
                            header::CONTENT_TYPE,
                            if name.ends_with(".wasm") {
                                "application/wasm"
                            } else if name.ends_with(".js") {
                                "text/javascript; charset=utf-8"
                            } else {
                                "application/octet-stream"
                            },
                        )],
                        bytes,
                    )
                        .into_response(),
                    None => StatusCode::NOT_FOUND.into_response(),
                }
            }),
        )
        .route(
            "/assets/pdf-reader.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("pdf-reader.js"),
                )
            }),
        )
        .route(
            "/assets/pdfjs/pdf.mjs",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../vendor/pdfjs/pdf.mjs"),
                )
            }),
        )
        .route(
            "/assets/pdfjs/pdf.worker.mjs",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("../vendor/pdfjs/pdf.worker.mjs"),
                )
            }),
        )
        .route(
            "/assets/theme.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("theme.js"),
                )
            }),
        )
        .route(
            "/assets/style.css",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/css; charset=utf-8")],
                    include_str!("style.css"),
                )
            }),
        )
        .route(
            "/assets/process.js",
            get(|| async {
                (
                    [(header::CONTENT_TYPE, "text/javascript; charset=utf-8")],
                    include_str!("process.js"),
                )
            }),
        )
        .layer(DefaultBodyLimit::max(64 * 1024))
        .layer(middleware::from_fn(security))
        .with_state(app)
}
async fn security(req: Request, next: Next) -> Response {
    let mut r = next.run(req).await;
    let h = r.headers_mut();
    for (k, v) in [
        ("cache-control", "no-store"),
        ("referrer-policy", "no-referrer"),
        ("x-content-type-options", "nosniff"),
        ("x-frame-options", "SAMEORIGIN"),
        (
            "content-security-policy",
            "default-src 'none'; script-src 'self' 'wasm-unsafe-eval'; worker-src 'self'; connect-src 'self'; style-src 'self'; img-src 'self' data:; frame-src 'self'; object-src 'none'; base-uri 'none'; form-action 'self'; frame-ancestors 'self'",
        ),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=()",
        ),
    ] {
        h.insert(
            axum::http::HeaderName::from_static(k),
            HeaderValue::from_static(v),
        );
    }
    r
}
async fn login_page(State(app): S) -> Response {
    let secret = db::token();
    let body = format!(
        "<section class='login panel'><span class='eyebrow'>AFSENDERPORTAL</span><h1>Godt at have det<br>på skrift.</h1><p class='muted'>Log ind for at sende dokumenter til underskrift og følge dem hele vejen.</p><form method='post' action='/login'>{}<label>Login<input type='text' name='email' autocomplete='username' required></label><label>Adgangskode<input type='password' name='password' autocomplete='current-password' required maxlength='1024'></label><button>Log ind <span>→</span></button></form></section>",
        csrf_field(&secret)
    );
    let mut r = Html(page("Log ind", &body, None)).into_response();
    r.headers_mut().insert(
        header::SET_COOKIE,
        set_cookie(&app, "login_nonce", &secret, 1800),
    );
    r
}
// Caddy overwrites this private header with its actual TCP peer address.
// Never trust client-supplied forwarding headers or non-loopback connections.
fn login_client_ip(config: &Config, peer: SocketAddr, h: &HeaderMap) -> Result<std::net::IpAddr> {
    if config.trust_proxy && peer.ip().is_loopback() {
        if h.get_all("x-skriv-under-client-ip").iter().count() != 1 {
            return Err(denied());
        }
        return h
            .get("x-skriv-under-client-ip")
            .and_then(|v| v.to_str().ok())
            .and_then(|s| s.parse().ok())
            .ok_or_else(denied);
    }
    Ok(peer.ip())
}
#[derive(Deserialize)]
struct Login {
    email: String,
    password: String,
    csrf: String,
}
async fn login(
    State(app): S,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    h: HeaderMap,
    Form(f): Form<Login>,
) -> Result<Response> {
    check_csrf(&cookie(&h, "login_nonce").ok_or_else(denied)?, &f.csrf)?;
    if f.email.len() > 254 || f.password.len() > 1024 {
        return Err(bad("Ugyldigt login"));
    }
    let ip = login_client_ip(&app.config, peer, &h)?.to_string();
    let mut tx = app.pool.begin().await?;
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM login_attempts WHERE ip=? AND at>?")
        .bind(&ip)
        .bind(db::now() - 900)
        .fetch_one(&mut *tx)
        .await?;
    if count >= 10 {
        return Err(Error(
            StatusCode::TOO_MANY_REQUESTS,
            "For mange loginforsøg. Prøv igen om 15 minutter.".into(),
        ));
    }
    sqlx::query("INSERT INTO login_attempts(ip,at) VALUES(?,?)")
        .bind(ip)
        .bind(db::now())
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query("SELECT id,password_hash FROM users WHERE email=?")
        .bind(f.email.trim().to_lowercase())
        .fetch_optional(&mut *tx)
        .await?;
    let fallback: String = sqlx::query_scalar("SELECT password_hash FROM users LIMIT 1")
        .fetch_one(&mut *tx)
        .await?;
    tx.commit().await?;
    let hash = row
        .as_ref()
        .map(|r| r.get::<String, _>("password_hash"))
        .unwrap_or(fallback);
    let verification_hash = hash.clone();
    let valid = tokio::task::spawn_blocking(move || {
        PasswordHash::new(&verification_hash).is_ok_and(|parsed| {
            Argon2::default()
                .verify_password(f.password.as_bytes(), &parsed)
                .is_ok()
        })
    })
    .await
    .unwrap_or(false);
    if !valid || row.is_none() {
        return Err(Error(
            StatusCode::UNAUTHORIZED,
            "Login eller adgangskode er forkert.".into(),
        ));
    }
    let user: i64 = row.unwrap().get("id");
    let secret = db::token();
    // Password verification runs outside SQLite. Recheck its exact version atomically
    // with session insertion, so configure-admin cannot be bypassed by an in-flight login.
    let inserted=sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at) SELECT ?,id,? FROM users WHERE id=? AND password_hash=?")
        .bind(db::hash(&secret)).bind(db::now()+28800).bind(user).bind(&hash).execute(&app.pool).await?;
    if inserted.rows_affected() != 1 {
        return Err(Error(
            StatusCode::UNAUTHORIZED,
            "Dit login er blevet ændret. Prøv igen med den aktuelle adgangskode.".into(),
        ));
    }
    let mut r = Redirect::to("/").into_response();
    r.headers_mut().append(
        header::SET_COOKIE,
        set_cookie(&app, "session", &secret, 28800),
    );
    r.headers_mut()
        .append(header::SET_COOKIE, set_cookie(&app, "login_nonce", "", 0));
    Ok(r)
}
#[derive(Deserialize)]
struct Csrf {
    csrf: String,
}
async fn logout(State(app): S, h: HeaderMap, Form(f): Form<Csrf>) -> Result<Response> {
    let (_, secret) = auth(&app, &h).await?;
    check_csrf(&secret, &f.csrf)?;
    sqlx::query("DELETE FROM sessions WHERE token_hash=?")
        .bind(db::hash(secret))
        .execute(&app.pool)
        .await?;
    let mut r = Redirect::to("/login").into_response();
    r.headers_mut()
        .insert(header::SET_COOKIE, set_cookie(&app, "session", "", 0));
    Ok(r)
}
#[derive(Default, Deserialize)]
struct ListFilter {
    #[serde(default)]
    q: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    page: usize,
}
fn list_url(archive: bool, q: &str, status: &str, page: usize) -> String {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("q", q)
        .append_pair("status", status)
        .append_pair("page", &page.to_string())
        .finish();
    format!("{}?{}", if archive { "/archive" } else { "/" }, query)
}
async fn dashboard(
    State(app): S,
    Query(filter): Query<ListFilter>,
    h: HeaderMap,
) -> Result<Response> {
    listing(&app, &h, filter, false).await
}
async fn archive(
    State(app): S,
    Query(filter): Query<ListFilter>,
    h: HeaderMap,
) -> Result<Response> {
    listing(&app, &h, filter, true).await
}
async fn listing(app: &App, h: &HeaderMap, filter: ListFilter, archive: bool) -> Result<Response> {
    let (user, secret) = match auth(app, h).await {
        Ok(v) => v,
        Err(_) => return Ok(Redirect::to("/login").into_response()),
    };
    let listing =
        admin::list_documents(app, user, &filter.q, &filter.status, filter.page, archive).await?;
    let pending = listing.pending;
    let completed = listing.completed;
    let mut body = if archive {
        format!(
            "<div class='heading'><div><span class='eyebrow'>DIT DOKUMENTARKIV</span><h1>Underskrevet og gemt.</h1><p class='muted'>Find aftalen igen. Hent den færdige PDF, kvitteringen og hele forløbet.</p></div><a class='button secondary' href='/'>Følg signeringsrunder →</a></div><aside class='notice archive-note'>Du har adgang her via dit administratorlogin, også efter at modtagernes downloadlinks er udløbet. {completed} signerede dokumenter er gemt.</aside>"
        )
    } else {
        format!(
            "<div class='heading'><div><span class='eyebrow'>DIT OVERBLIK</span><h1>Aftaler, der kommer videre.</h1><p class='muted'>Følg signeringslinks og underskrifter, og se hvem vi afventer.</p></div><a class='button' href='/new'>+ Nyt dokument</a></div><div class='stats'><div><span>Dokumenter</span><strong>{}</strong></div><div><span>Afventer underskrift</span><strong>{pending}</strong></div><div><a href='/archive'><span>I signeret arkiv</span><strong>{completed}</strong></a></div></div>",
            listing.all_count
        )
    };
    let target = if archive { "/archive" } else { "/" };
    body.push_str(&format!("<section class='panel'><div class='section-title'><h2>{}</h2><span>Nyeste først</span></div><form class='filters' role='search' method='get' action='{target}'><label>Søg dokumenter<input name='q' value='{}' maxlength='200' placeholder='Titel eller navn'></label>",if archive {"Signerede dokumenter"} else {"Dine dokumenter"},esc(&filter.q)));
    if !archive {
        body.push_str("<label>Status<select name='status'>");
        for (value, label) in [
            ("", "Alle statusser"),
            ("pending", "Afventer"),
            ("draft", "Kladder"),
            ("completed", "Gennemført"),
            ("declined", "Afvist"),
            ("expired", "Udløbet"),
            ("cancelled", "Annulleret"),
        ] {
            body.push_str(&format!(
                "<option value='{value}'{}>{label}</option>",
                if filter.status == value {
                    " selected"
                } else {
                    ""
                }
            ));
        }
        body.push_str("</select></label>");
    }
    body.push_str(&format!(
        "<button>Søg</button><a href='{target}'>Nulstil</a></form>"
    ));
    let filtered = &listing.documents;
    let pages = listing.pages;
    let current_page = listing.page;
    body.push_str(&format!(
        "<p class='muted result-count'>{} dokumenter fundet</p>",
        listing.total
    ));
    if filtered.is_empty() {
        body.push_str(if archive {"<div class='empty'><h2>Ingen signerede dokumenter fundet</h2><p>Dokumenter vises automatisk her, når alle har underskrevet.</p></div>"} else {"<div class='empty'><h2>Ingen dokumenter fundet</h2><p>Prøv en anden søgning, eller opret en ny signeringsrunde.</p><a class='button' href='/new'>Opret dokument</a></div>"});
    } else {
        body.push_str(&format!("<div class='table-wrap'><table><thead><tr><th>Dokument</th><th>{}</th><th>{}</th><th>Handlinger</th></tr></thead><tbody>",if archive {"Underskrifter"} else {"Status og fremdrift"},if archive {"Afsluttet"} else {"Afventer / oprettet"}));
        for d in filtered.iter() {
            let status = if archive {
                String::new()
            } else {
                badge(&d.status)
            };
            let info = if archive {
                d.completed_at
                    .map(db::date)
                    .unwrap_or_else(|| "Gennemført".into())
            } else if d.status == "pending" {
                format!("Afventer: {}", esc(d.awaiting.as_deref().unwrap_or("")))
            } else {
                db::date(d.created_at)
            };
            let downloads = if d.status == "completed" {
                format!(
                    "<a href='/documents/{}/files/final'>Signeret PDF ↓</a><a href='/documents/{}/files/receipt'>Kvittering ↓</a>",
                    d.id, d.id
                )
            } else {
                String::new()
            };
            body.push_str(&format!("<tr><td><a class='doc-title' href='/documents/{}'>▤ &nbsp;{}</a><small>{} {}</small></td><td>{status}<small>{} af {} underskrevet</small><progress value='{}' max='{}' aria-label='Underskrifter for {}'></progress></td><td class='muted'>{info}</td><td class='row-actions'><a href='/documents/{}'>Se forløb →</a>{downloads}</td></tr>",d.id,esc(&d.title),d.file_count,if d.file_count==1 {"dokument"} else {"dokumenter i pakken"},d.signed,d.total,d.signed,d.total.max(1),esc(&d.title),d.id));
        }
        body.push_str("</tbody></table></div>");
    }
    if pages > 1 {
        body.push_str("<nav class='pagination' aria-label='Dokumentsider'>");
        if current_page > 1 {
            body.push_str(&format!(
                "<a href='{}'>← Forrige</a>",
                esc(&list_url(
                    archive,
                    &filter.q,
                    &filter.status,
                    current_page - 1
                ))
            ));
        }
        body.push_str(&format!("<span>Side {current_page} af {pages}</span>"));
        if current_page < pages {
            body.push_str(&format!(
                "<a href='{}'>Næste →</a>",
                esc(&list_url(
                    archive,
                    &filter.q,
                    &filter.status,
                    current_page + 1
                ))
            ));
        }
        body.push_str("</nav>");
    }
    body.push_str("</section>");
    Ok(Html(page(
        if archive {
            "Signeret arkiv"
        } else {
            "Dokumenter"
        },
        &body,
        Some(&secret),
    ))
    .into_response())
}

async fn process_status(State(app): S, Path(id): Path<i64>, h: HeaderMap) -> Result<Response> {
    let (user, _) = auth(&app, &h).await?;
    owned(&app, id, user).await?;
    db::expire(&app.pool).await?;
    let row=sqlx::query("SELECT status,(SELECT coalesce(max(id),0) FROM events WHERE document_id=documents.id) AS revision FROM documents WHERE id=?").bind(id).fetch_one(&app.pool).await?;
    Ok(axum::Json(serde_json::json!({"status":row.get::<String,_>("status"),"revision":row.get::<i64,_>("revision")})).into_response())
}
fn event_label(kind: &str) -> &str {
    match kind {
        "created" => "Kladde oprettet",
        "sent" => "Tidligere afsendt til underskrift",
        "started" => "Signeringslinks klar",
        "link_created" => "Personligt signeringslink oprettet",
        "links_rotated" => "Signeringslinks udskiftet",
        "download_links_renewed" => "Læselinks udskiftet",
        "download_link_created" => "Læselink til færdig aftale oprettet",
        "invitation_queued" => "Invitation oprettet",
        "email_sent" => "E-mail accepteret af mailserver",
        "email_local" => "E-mail gemt i lokal udbakke",
        "email_failed" => "E-maillevering fejlede · genforsøg planlagt",
        "link_opened" => "Invitationslink åbnet",
        "document_accessed" => "PDF tilgået",
        "signed" => "Underskrift registreret",
        "completed" => "Alle har underskrevet · gemt i arkiv",
        "declined" => "Dokument afvist",
        "cancelled" => "Signeringsrunde annulleret",
        "expired" => "Signeringsfrist udløbet",
        _ => kind,
    }
}
fn package_manifest(files: &[db::SourceFile], base: &str) -> String {
    if files.len() < 2 {
        return String::new();
    }
    let mut html = format!(
        "<section class='panel package-manifest'><div class='section-title'><h2>Dokumenter i pakken</h2><span>{} dokumenter · én underskrift</span></div><p class='muted'>Din underskrift omfatter alle dokumenterne nedenfor, i denne rækkefølge.</p><ol>",
        files.len()
    );
    for f in files {
        let url = if base == "/sign" {
            format!("/sign/document.pdf?source={}", f.position)
        } else {
            format!("{base}/files/source-{}", f.position)
        };
        html.push_str(&format!("<li><span class='file-number'>{:02}</span><div><a href='{}' target='_blank' rel='noopener'>{} ↗</a><small>{} {} · side {}–{} i pakken</small><details><summary>Dokumentets SHA-256</summary><code class='hash'>{}</code></details></div></li>", f.position, url, esc(&f.name), f.pages, if f.pages == 1 {"side"} else {"sider"}, f.first_page, f.first_page+f.pages-1, f.sha256));
    }
    html.push_str("</ol></section>");
    html
}
async fn new_page(State(app): S, h: HeaderMap) -> Result<Html<String>> {
    let (_, secret) = auth(&app, &h).await?;
    let body = format!(
        "<a class='back' href='/'>← Alle dokumenter</a><div class='heading'><div><span class='eyebrow'>NY SIGNERINGSRUNDE</span><h1>En aftale. Alle dokumenterne.</h1><p class='muted'>Saml dokumenterne, tilføj underskriverne, og gennemse pakken før du deler links.</p></div></div><form class='panel editor' method='post' action='/documents' enctype='multipart/form-data'>{}<div class='form-step'><span>1</span><div><h2>Dokumentpakken</h2><p>Ét personligt link. Én samlet underskrift.</p></div></div><label>Titel<input name='title' required maxlength='150' placeholder='Fx Samarbejdsaftale'></label><label for='package-pdfs'>PDF-dokumenter</label><div class='upload-area'><strong>Vælg dokumenterne til din aftale</strong><p>Fx hovedaftale, vilkår og bilag. Alle underskrives samlet.</p><input id='package-pdfs' type='file' name='pdf' accept='application/pdf,.pdf' multiple required aria-describedby='upload-help'><small id='upload-help'>1–10 PDF-filer. Tilsammen højst 20 MiB og 200 sider, uden adgangskode eller aktivt indhold.</small><ol class='upload-list' data-upload-list aria-live='polite'></ol></div><label>Besked på signeringssiden<textarea name='message' maxlength='4000' rows='3' placeholder='En kort besked om dokumentet'></textarea></label><hr><div class='form-step'><span>2</span><div><h2>Hvem skal underskrive?</h2><p>Hver person får ét link til hele pakken.</p></div></div><label>Underskrivernes navne — én person pr. linje<textarea name='recipients' required maxlength='10000' rows='4' placeholder='Anna Jensen&#10;Bo Hansen'></textarea><small>Højst 20 personer. Du får ét personligt link pr. person og deler selv linkene.</small></label><label>Frist efter oprettelse af links<select name='days'><option value='7'>7 dage</option><option value='14' selected>14 dage</option><option value='30'>30 dage</option></select></label><div class='form-actions'><a href='/'>Annullér</a><button>Gem kladde →</button></div></form>",
        csrf_field(&secret)
    );
    Ok(Html(page("Nyt dokument", &body, Some(&secret))))
}
async fn upload(State(app): S, h: HeaderMap, mut form: Multipart) -> Result<Redirect> {
    let (user, secret) = auth(&app, &h).await?;
    let mut values = std::collections::HashMap::new();
    let mut files = Vec::new();
    while let Some(field) = form
        .next_field()
        .await
        .map_err(|_| bad("Upload kunne ikke læses"))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "pdf" {
            if files.len() >= 10 {
                return Err(bad("Vælg højst 10 PDF-filer"));
            }
            let filename = field.file_name().unwrap_or("Dokument.pdf").to_string();
            let bytes = field
                .bytes()
                .await
                .map_err(|_| bad("PDF kunne ikke læses"))?
                .to_vec();
            files.push(admin::UploadFile {
                name: filename,
                bytes,
            });
        } else {
            let value = field.text().await.map_err(|_| bad("Ugyldigt felt"))?;
            if value.len() > 16000 {
                return Err(bad("Et felt er for langt"));
            }
            values.insert(name, value);
        }
    }
    let value = |k: &str| values.get(k).map(String::as_str).unwrap_or("");
    check_csrf(&secret, value("csrf"))?;
    let id = admin::create_package(
        &app,
        user,
        "afsender",
        admin::NewDocument {
            title: value("title").into(),
            message: value("message").into(),
            recipients: value("recipients").into(),
            days: value("days").parse().map_err(|_| bad("Ugyldig frist"))?,
            pdf: Vec::new(),
        },
        files,
    )
    .await?;
    Ok(Redirect::to(&format!("/documents/{id}")))
}

async fn detail(State(app): S, Path(id): Path<i64>, h: HeaderMap) -> Result<Html<String>> {
    let (user, secret) = auth(&app, &h).await?;
    db::expire(&app.pool).await?;
    let revision: i64 =
        sqlx::query_scalar("SELECT coalesce(max(id),0) FROM events WHERE document_id=?")
            .bind(id)
            .fetch_one(&app.pool)
            .await?;
    let d = owned(&app, id, user).await?;
    let people = recipients(&app, id).await?;
    let mut body = format!(
        "<a class='back' href='/'>← Alle dokumenter</a><div class='heading'><div><span class='eyebrow'>DOKUMENT #{id}</span><h1>{}</h1>{}</div><a class='button secondary' href='/documents/{id}/files/original'>Hent original PDF ↓</a></div>",
        esc(&d.title),
        badge(&d.status)
    );
    let files = db::source_files(&app.pool, id).await?;
    body.push_str(&package_manifest(&files, &format!("/documents/{id}")));
    body.push_str("<div class='detail-grid'><section class='panel'><h2>Underskrivere</h2><p class='muted'>Alle underskriver samme låste dokumentversion.</p>");
    let signed = people.iter().filter(|r| r.status == "signed").count();
    body.push_str(&format!("<div class='process-summary' data-process-id='{id}' data-revision='{revision}' data-status='{}'><p><strong>{signed} af {} underskrevet</strong></p><progress value='{signed}' max='{}' aria-label='Samlet signeringsfremdrift'></progress><small data-live-note>{}</small></div>",esc(&d.status),people.len(),people.len().max(1),if d.status=="pending" {"Status opdateres automatisk hvert 10. sekund."}else if d.status=="completed" {"Alle har underskrevet. Dokumentet er gemt i arkivet."} else {"Rundens hændelser er registreret i revisionssporet."}));
    if d.status == "pending" {
        let waiting = people
            .iter()
            .filter(|r| r.status == "pending")
            .map(|r| esc(&r.name))
            .collect::<Vec<_>>()
            .join(", ");
        body.push_str(&format!("<p class='waiting'>Afventer: {waiting}</p>"));
    }
    for r in &people {
        body.push_str(&format!("<div class='person'><div class='avatar'>{}</div><div class='person-info'><strong>{}</strong>{}</div>{}</div>",esc(&r.name.chars().next().unwrap_or('?').to_string()),esc(&r.name),r.signed_at.map(|t|format!("<small>{}</small>",db::date(t))).unwrap_or_default(),badge(&r.status)));
        if d.status == "pending" && r.status == "pending" {
            let nonce: Option<String> =
                sqlx::query_scalar("SELECT nonce FROM recipient_links WHERE recipient_id=?")
                    .bind(r.id)
                    .fetch_optional(&app.pool)
                    .await?;
            if let Some(nonce) = nonce {
                let token = links::token(&app.link_key, "recipient", r.id, &nonce);
                body.push_str(&copy_link(
                    &format!("signer-{}", r.id),
                    &format!("{}/s/{token}", app.config.base_url),
                    &format!("Personligt signeringslink til {}", r.name),
                ));
            } else {
                body.push_str("<p class='muted'>Opret nye links for at kunne kopiere et link her. Det erstatter tidligere links.</p>");
            }
        }
        let access=sqlx::query("SELECT max(CASE WHEN kind IN ('link_created','invitation_queued') THEN at END) AS created, max(CASE WHEN kind='link_opened' THEN at END) AS opened, max(CASE WHEN kind='document_accessed' THEN at END) AS accessed FROM events WHERE document_id=? AND (actor=? OR actor=?)")
            .bind(id).bind(r.actor()).bind(r.email.as_deref().unwrap_or("")).fetch_one(&app.pool).await?;
        body.push_str("<div class='recipient-progress'>");
        if let Some(at) = access.get::<Option<i64>, _>("created") {
            body.push_str(&format!("<small>Link oprettet: {}</small>", db::date(at)));
        }
        if let Some(at) = access.get::<Option<i64>, _>("opened") {
            body.push_str(&format!(
                "<small>Link åbnet (kan være automatisk): {}</small>",
                db::date(at)
            ));
        }
        if let Some(at) = access.get::<Option<i64>, _>("accessed") {
            body.push_str(&format!(
                "<small>PDF tilgået (ikke læsebevis): {}</small>",
                db::date(at)
            ));
        }
        body.push_str("</div>");
        if let Some(reason) = &r.reason {
            body.push_str(&format!("<p>Begrundelse: {}</p>", esc(reason)));
        }
    }
    body.push_str("</section><aside class='panel'><h2>Næste skridt</h2>");
    if d.status == "draft" {
        body.push_str(&format!("<p>Gennemse dokumentet og underskriverne. Opret derefter personlige links, som du selv deler. Fristen starter ved oprettelsen.</p><form method='post' action='/documents/{id}/start'>{}<button>Opret signeringslinks →</button></form><p class='muted'>Frist: {} dage efter oprettelse af links.</p>",csrf_field(&secret),d.expires_at/86400));
    }
    if d.status == "pending" {
        body.push_str(&format!("<p>Frist: {}</p><form method='post' action='/documents/{id}/rotate'>{}<button>Erstat links til afventende</button></form><p class='muted'>Nye links erstatter de tidligere links. Mindst 60 sekunder mellem linkudskiftninger.</p>",db::date(d.expires_at),csrf_field(&secret)));
    }
    if ["draft", "pending"].contains(&d.status.as_str()) {
        body.push_str(&format!("<form method='post' action='/documents/{id}/cancel'>{}<button class='danger'>Annullér signeringsrunde</button></form>",csrf_field(&secret)));
    }
    if d.status == "completed" {
        body.push_str(&format!("<p>Alle har underskrevet. Dokumentet er gemt i dit signerede arkiv. Du kan hente det her, også efter at modtagernes downloadlinks er udløbet.</p><a class='text-link' href='/archive'>Åbn signeret arkiv →</a><a class='button' href='/documents/{id}/files/final'>Hent færdig PDF ↓</a><a class='text-link' href='/documents/{id}/files/receipt'>Hent signeringskvittering</a>"));
    }
    if d.status == "completed" {
        let shared =
            sqlx::query("SELECT nonce,expires_at FROM shared_download_links WHERE document_id=?")
                .bind(id)
                .fetch_optional(&app.pool)
                .await?;
        if let Some(shared) = shared {
            let expires: i64 = shared.get("expires_at");
            if expires > db::now() {
                let token = links::token(
                    &app.link_key,
                    "download",
                    id,
                    &shared.get::<String, _>("nonce"),
                );
                body.push_str("<h3>Del den færdige aftale</h3><p class='muted'>Læselinks giver kun adgang til denne færdige aftale og kvittering.</p>");
                body.push_str(&copy_link(
                    "final-link",
                    &format!("{}/d/{token}/final", app.config.base_url),
                    "Link til signeret PDF",
                ));
                body.push_str(&copy_link(
                    "receipt-link",
                    &format!("{}/d/{token}/receipt", app.config.base_url),
                    "Link til kvittering",
                ));
                body.push_str(&format!("<small>Udløber: {}</small>", db::date(expires)));
            } else {
                body.push_str("<p>De delte læselinks er udløbet. Du kan oprette nye her.</p>");
            }
        }
        body.push_str(&format!("<form method='post' action='/documents/{id}/renew-download'>{}<button class='secondary'>Opret / erstat læselinks</button></form>",csrf_field(&secret)));
    }
    if ["cancelled", "expired", "declined"].contains(&d.status.as_str()) {
        body.push_str("<p>Denne signeringsrunde er lukket. Opret en ny runde for at indhente underskrifter.</p>");
    }
    body.push_str("</aside></div>");
    if !d.message.is_empty() {
        body.push_str(&format!("<section class='panel'><h2>Besked på signeringssiden</h2><p class='preserve'>{}</p></section>",esc(&d.message)));
    }
    body.push_str(&format!("<section class='panel'><h2>Dokumentversion</h2><p class='muted'>SHA-256 af den oprindelige PDF</p><code class='hash'>{}</code><p class='muted'>Oprettet {}</p><a href='/documents/{id}/audit.json'>Hent revisionsspor som JSON ↓</a></section>",esc(&d.original_hash),db::date(d.created_at)));
    body.push_str("<section class='panel'><h2>Revisionsspor</h2><ol class='timeline'>");
    let events = sqlx::query("SELECT * FROM events WHERE document_id=? ORDER BY id DESC")
        .bind(id)
        .fetch_all(&app.pool)
        .await?;
    for e in events {
        body.push_str(&format!(
            "<li><strong>{}</strong><p>{}</p><small>{} · {}</small></li>",
            esc(event_label(&e.get::<String, _>("kind"))),
            esc(&e.get::<String, _>("detail")),
            esc(&people
                .iter()
                .find(|r| r.actor() == e.get::<String, _>("actor")
                    || r.email.as_deref() == Some(e.get::<String, _>("actor").as_str()))
                .map(|r| r.name.clone())
                .unwrap_or_else(|| e.get("actor"))),
            db::date(e.get("at"))
        ));
    }
    body.push_str("</ol></section>");
    Ok(Html(page(&d.title, &body, Some(&secret))))
}
async fn document_action(
    State(app): S,
    Path((id, action)): Path<(i64, String)>,
    h: HeaderMap,
    Form(f): Form<Csrf>,
) -> Result<Redirect> {
    let (user, secret) = auth(&app, &h).await?;
    check_csrf(&secret, &f.csrf)?;
    admin::change_document(&app, user, "afsender", id, &action).await?;
    Ok(Redirect::to(&format!("/documents/{id}")))
}

async fn open_invitation(State(app): S, Path(token): Path<String>) -> Result<Response> {
    if token.len() != 64 {
        return Err(denied());
    }
    let r: Recipient = sqlx::query_as("SELECT * FROM recipients WHERE token_hash=?")
        .bind(db::hash(&token))
        .fetch_optional(&app.pool)
        .await?
        .ok_or_else(denied)?;
    // GET only opens a view session. It neither signs nor consumes the invitation.
    let mut response = Redirect::to("/sign").into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        set_cookie(&app, "signer", &token, 86400),
    );
    // A link open may be a mail scanner; do not claim that the recipient read it.
    let mut tx = app.pool.begin().await?;
    db::access_event(
        &mut tx,
        r.document_id,
        "link_opened",
        &r.actor(),
        "Signeringslink åbnet; kan være automatisk",
    )
    .await?;
    tx.commit().await?;
    Ok(response)
}
fn pdf_reader(files: &[db::SourceFile]) -> String {
    let mut html = "<section class='panel pdf-reader' id='pdf-reader' aria-label='Læs dokumenterne'><h2>Læs før du underskriver</h2><label for='reader-document'>Dokument</label><select id='reader-document'>".to_string();
    if files.is_empty() {
        html.push_str("<option value='/sign/document.pdf'>Dokument til underskrift</option>");
    } else {
        for f in files {
            html.push_str(&format!(
                "<option value='/sign/document.pdf?source={}'>{}. {}</option>",
                f.position,
                f.position,
                esc(&f.name)
            ));
        }
    }
    html.push_str("</select><div class='reader-toolbar' role='group' aria-label='PDF-betjening'><button type='button' class='secondary small' data-reader-prev disabled>← Forrige side</button><span data-reader-page>Side –</span><button type='button' class='secondary small' data-reader-next disabled>Næste side →</button><label for='reader-zoom'>Zoom<select id='reader-zoom'><option value='1'>Tilpas bredde</option><option value='1.5'>150 %</option><option value='2'>200 %</option></select></label></div><p class='muted' data-reader-status role='status'>Indlæser PDF …</p><button type='button' class='secondary small' data-reader-retry hidden>Prøv igen</button><div class='reader-canvas' tabindex='0' aria-label='PDF-side. Brug sideskift ovenfor.'><canvas role='img' aria-label='Dokumentside'></canvas></div><details class='reader-text'><summary>Vis sidens tekst</summary><div data-reader-text class='preserve'></div></details><p><a data-reader-download href='/sign/document.pdf' target='_blank' rel='noopener'>Åbn den valgte PDF i en ny fane ↗</a></p><noscript><p>Aktivér JavaScript for at læse PDF’en her, eller åbn PDF-linket ovenfor.</p></noscript></section><script type='module' src='/assets/pdf-reader.js'></script>");
    html
}
async fn sign_page(State(app): S, h: HeaderMap) -> Result<Html<String>> {
    db::expire(&app.pool).await?;
    let (r, d, secret) = signer(&app, &h).await?;
    let files = db::source_files(&app.pool, d.id).await?;
    let is_package = files.len() > 1;
    let consent = if is_package {
        db::PACKAGE_CONSENT
    } else {
        db::CONSENT
    };
    let mut body = format!(
        "<div class='heading'><div><span class='eyebrow'>PERSONLIG INVITATION TIL {}</span><h1>{}</h1>{}</div></div>",
        esc(&r.name),
        esc(&d.title),
        badge(&d.status)
    );
    if ["pending", "completed"].contains(&d.status.as_str()) {
        body.push_str(&package_manifest(&files, "/sign"));
    }
    if d.status == "completed" {
        body.push_str("<section class='panel success'><h2>Alle underskrifter er på plads.</h2><p>Tak. Du kan nu hente det færdige dokument og signeringskvitteringen.</p><a class='button' href='/sign/document.pdf'>Hent færdig PDF ↓</a><a class='text-link' href='/sign/receipt.pdf'>Hent kvittering</a></section>");
    } else if d.status != "pending" {
        body.push_str("<section class='panel'><h2>Signeringsrunden er lukket</h2><p>Du kan ikke længere underskrive. Kontakt afsenderen for en ny invitation.</p></section>");
    } else {
        body.push_str(&format!("<section class='panel'><p class='preserve'>{}</p><p class='muted'>Underskriver: {} · Frist: {}</p><a href='/sign/document.pdf' target='_blank' rel='noopener'>Åbn PDF i en ny fane ↗</a></section>",esc(&d.message),esc(&r.name),db::date(d.expires_at)));
        body.push_str(&pdf_reader(&files));
        if r.status == "signed" {
            body.push_str("<section class='panel success'><h2>Din underskrift er registreret.</h2><p>Vi afventer de øvrige underskrivere. Brug dit personlige link igen for at hente den færdige aftale, når alle har underskrevet.</p></section>");
        } else {
            body.push_str(&format!("<section class='panel signing'><span class='eyebrow'>DIN BEKRÆFTELSE</span><h2>Klar til at underskrive?</h2><p>Gennemgå alle dokumenter ovenfor. Du underskriver den samlede dokumentversion med SHA-256:</p><code class='hash'>{}</code><form method='post' action='/sign'>{}<input type='hidden' name='action' value='sign'><label>Dit fulde navn<input name='name' value='{}' required minlength='2' maxlength='120' autocomplete='name'></label><label class='checkbox'><input type='checkbox' name='agree' value='yes' required><span>{}</span></label><p class='muted'>Du bekræfter med det personlige link, du har modtaget fra afsenderen. Linket verificerer ikke din identitet. Der foretages ingen MitID-kontrol.</p><button>{}</button></form><details><summary>Jeg ønsker at afvise dokumentet</summary><form method='post' action='/sign'>{}<input type='hidden' name='action' value='decline'><label>Begrundelse<textarea name='reason' required maxlength='1000' rows='3'></textarea></label><button class='danger'>Afvis dokumentet</button></form></details></section>",d.original_hash,csrf_field(&secret),esc(&r.name),esc(consent),if is_package {format!("Underskriv alle {} dokumenter ✓",files.len())} else {"Underskriv dokumentet ✓".into()},csrf_field(&secret)));
        }
    }
    Ok(Html(page(&d.title, &body, None)))
}
#[derive(Deserialize)]
struct SignForm {
    csrf: String,
    action: String,
    name: Option<String>,
    agree: Option<String>,
    reason: Option<String>,
}
async fn sign(State(app): S, h: HeaderMap, Form(f): Form<SignForm>) -> Result<Redirect> {
    let (r, _, secret) = signer(&app, &h).await?;
    check_csrf(&secret, &f.csrf)?;
    let mut tx = app.pool.begin().await?;
    let d: Document = sqlx::query_as("SELECT id,owner_id,title,message,status,original_hash,created_at,expires_at FROM documents WHERE id=?")
        .bind(r.document_id)
        .fetch_one(&mut *tx)
        .await?;
    let current: Recipient = sqlx::query_as("SELECT * FROM recipients WHERE id=? AND token_hash=?")
        .bind(r.id)
        .bind(db::hash(&secret))
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(denied)?;
    if d.status != "pending" || d.expires_at <= db::now() || current.status != "pending" {
        return Err(bad(
            "Dokumentet er lukket, fristen er udløbet, eller du har allerede svaret",
        ));
    }
    let sources: Vec<db::SourceFile> = sqlx::query_as("SELECT position,name,sha256,pages,first_page FROM document_files WHERE document_id=? ORDER BY position").bind(d.id).fetch_all(&mut *tx).await?;
    let consent = if sources.len() > 1 {
        db::PACKAGE_CONSENT
    } else {
        db::CONSENT
    };
    match f.action.as_str() {
        "decline" => {
            let reason = f.reason.unwrap_or_default();
            let reason = reason.trim();
            if reason.is_empty() || reason.chars().count() > 1000 {
                return Err(bad("Angiv en begrundelse på højst 1000 tegn"));
            }
            sqlx::query("UPDATE recipients SET status='declined',reason=? WHERE id=?")
                .bind(reason)
                .bind(r.id)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE documents SET status='declined' WHERE id=?")
                .bind(d.id)
                .execute(&mut *tx)
                .await?;
            db::event(&mut tx, d.id, "declined", &r.actor(), reason).await?;
        }
        "sign" => {
            let name = f.name.unwrap_or_default();
            let name = name.trim();
            if name.chars().count() < 2
                || name.chars().count() > 120
                || name.chars().any(char::is_control)
                || f.agree.as_deref() != Some("yes")
            {
                return Err(bad("Angiv dit fulde navn og bekræft samtykket"));
            }
            sqlx::query("UPDATE recipients SET status='signed',signed_name=?,signed_at=?,consent=? WHERE id=?").bind(name).bind(db::now()).bind(consent).bind(r.id).execute(&mut *tx).await?;
            db::event(
                &mut tx,
                d.id,
                "signed",
                &r.actor(),
                &format!(
                    "Navn: {name}. SHA-256: {}. Samtykke: {}",
                    d.original_hash, consent
                ),
            )
            .await?;
            let remaining: i64 = sqlx::query_scalar(
                "SELECT count(*) FROM recipients WHERE document_id=? AND status!='signed'",
            )
            .bind(d.id)
            .fetch_one(&mut *tx)
            .await?;
            if remaining == 0 {
                let people: Vec<Recipient> =
                    sqlx::query_as("SELECT * FROM recipients WHERE document_id=? ORDER BY id")
                        .bind(d.id)
                        .fetch_all(&mut *tx)
                        .await?;
                let original: Vec<u8> = sqlx::query_scalar("SELECT pdf FROM documents WHERE id=?")
                    .bind(d.id)
                    .fetch_one(&mut *tx)
                    .await?;
                let document_id = d.id;
                let title = d.title.clone();
                let hash = d.original_hash.clone();
                let ps = people.clone();
                let (final_pdf, receipt) = tokio::task::spawn_blocking(move || {
                    pdf::complete_package(&original, document_id, &title, &hash, &ps, &sources)
                })
                .await
                .map_err(|_| bad("PDF-behandlingen fejlede; underskriften er ikke gemt"))?
                .map_err(|_| bad("PDF kunne ikke færdiggøres; underskriften er ikke gemt"))?;
                let final_hash = db::hash(&final_pdf);
                sqlx::query("UPDATE documents SET status='completed',final_pdf=?,receipt_pdf=?,final_hash=? WHERE id=?").bind(final_pdf).bind(receipt).bind(&final_hash).bind(d.id).execute(&mut *tx).await?;
                db::event(
                    &mut tx,
                    d.id,
                    "completed",
                    "system",
                    &format!("Alle har underskrevet. SHA-256 for færdig PDF: {final_hash}"),
                )
                .await?;
                links::rotate_download(&mut tx, &app.link_key, d.id).await?;
            }
        }
        _ => return Err(bad("Ugyldig handling")),
    }
    tx.commit().await?;
    Ok(Redirect::to("/sign"))
}
fn pdf_response(bytes: Vec<u8>, filename: &str, inline: bool) -> Response {
    let mut r = bytes.into_response();
    r.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static("application/pdf"),
    );
    r.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "{}; filename=\"{filename}\"",
            if inline { "inline" } else { "attachment" }
        ))
        .unwrap(),
    );
    r
}
async fn file(app: &App, id: i64, kind: &str, inline: bool) -> Result<Response> {
    let bytes = admin::file_bytes(app, id, kind).await?;
    Ok(pdf_response(
        bytes,
        &format!("document-{id}-{kind}.pdf"),
        inline,
    ))
}
async fn owner_file(
    State(app): S,
    Path((id, kind)): Path<(i64, String)>,
    h: HeaderMap,
) -> Result<Response> {
    let (user, _) = auth(&app, &h).await?;
    owned(&app, id, user).await?;
    file(&app, id, &kind, false).await
}
#[derive(Deserialize)]
struct SourceQuery {
    source: Option<u32>,
}
async fn sign_file(
    State(app): S,
    h: HeaderMap,
    Query(query): Query<SourceQuery>,
) -> Result<Response> {
    let (r, d, _) = signer(&app, &h).await?;
    if !["pending", "completed"].contains(&d.status.as_str())
        || d.status == "pending" && d.expires_at <= db::now()
    {
        return Err(denied());
    }
    let mut tx = app.pool.begin().await?;
    db::access_event(
        &mut tx,
        d.id,
        "document_accessed",
        &r.actor(),
        "PDF hentet eller vist; dokumenterer ikke læsning",
    )
    .await?;
    tx.commit().await?;
    if let Some(position) = query.source {
        return file(&app, d.id, &format!("source-{position}"), true).await;
    }
    file(
        &app,
        d.id,
        if d.status == "completed" {
            "final"
        } else {
            "original"
        },
        true,
    )
    .await
}
async fn sign_receipt(State(app): S, h: HeaderMap) -> Result<Response> {
    let (_, d, _) = signer(&app, &h).await?;
    if d.status != "completed" {
        return Err(denied());
    }
    file(&app, d.id, "receipt", false).await
}
async fn download(State(app): S, Path((token, kind)): Path<(String, String)>) -> Result<Response> {
    if token.len() != 64 || !["final", "receipt"].contains(&kind.as_str()) {
        return Err(denied());
    }
    let id:Option<i64>=sqlx::query_scalar("SELECT x.document_id FROM downloads x JOIN documents d ON d.id=x.document_id WHERE x.token_hash=? AND x.expires_at>? AND d.status='completed'").bind(db::hash(token)).bind(db::now()).fetch_optional(&app.pool).await?;
    file(&app, id.ok_or_else(denied)?, &kind, false).await
}
async fn audit_json(State(app): S, Path(id): Path<i64>, h: HeaderMap) -> Result<Response> {
    let (user, _) = auth(&app, &h).await?;
    let mut response = axum::Json(admin::audit(&app, user, id).await?).into_response();
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_static("attachment; filename=\"audit.json\""),
    );
    Ok(response)
}

async fn integrations_page(State(app): S, h: HeaderMap) -> Result<Html<String>> {
    let (user, secret) = auth(&app, &h).await?;
    integrations(&app, user, &secret, None).await
}
#[derive(Deserialize)]
struct CreateTokenForm {
    csrf: String,
    name: String,
    days: i64,
}
async fn create_mcp_token(
    State(app): S,
    h: HeaderMap,
    Form(f): Form<CreateTokenForm>,
) -> Result<Html<String>> {
    let (user, secret) = auth(&app, &h).await?;
    check_csrf(&secret, &f.csrf)?;
    let created =
        mcp::create_token(&app, user, &f.name, f.days, mcp::Issuer::Session(&secret)).await?;
    integrations(&app, user, &secret, Some(created)).await
}
async fn revoke_mcp_token(
    State(app): S,
    Path(id): Path<i64>,
    h: HeaderMap,
    Form(f): Form<Csrf>,
) -> Result<Redirect> {
    let (user, secret) = auth(&app, &h).await?;
    check_csrf(&secret, &f.csrf)?;
    mcp::revoke_token(&app, user, id, None).await?;
    Ok(Redirect::to("/integrations"))
}
async fn integrations(
    app: &App,
    user: i64,
    secret: &str,
    created: Option<serde_json::Value>,
) -> Result<Html<String>> {
    let data = mcp::tokens(app, user).await?;
    let endpoint = format!("{}/mcp", app.config.base_url);
    let mut body = "<div class='heading'><div><span class='eyebrow'>DINE INTEGRATIONER</span><h1>AI med din adgang.</h1><p class='muted'>Forbind en AI-assistent til Skriv Under gennem MCP.</p></div></div>".to_string();
    if let Some(created) = created {
        body.push_str("<section class='panel success'><h2>Dit nye token er klar</h2><p>Gem tokenet nu. Det vises kun denne ene gang og giver fuld administratoradgang.</p>");
        body.push_str(&copy_link(
            "new-mcp-token",
            created["token"].as_str().unwrap(),
            "Hemmeligt MCP-token",
        ));
        body.push_str(&format!("<p>Udløber {}. Hvis du mister tokenet, kan du tilbagekalde det og oprette et nyt.</p></section>", db::date(created["expires_at"].as_i64().unwrap())));
    }
    body.push_str("<section class='panel'><h2>Forbind din AI</h2><p>Vælg en MCP-klient med Streamable HTTP og en brugerdefineret Authorization-header.</p>");
    body.push_str(&copy_link(
        "mcp-endpoint",
        &endpoint,
        "MCP-serverens adresse",
    ));
    body.push_str("<p>Angiv dit hemmelige token som Bearer-token i klientens adgangsindstillinger. Headeren er <code>Authorization: Bearer DIT_TOKEN</code>.</p><aside class='notice'>Tokenet giver samme adgang som dig: dokumenter, PDF'er, signeringslinks, annullering og oprettelse af nye admin-tokens. Del det kun med en AI-klient, du har valgt. Underskrivernes samtykke foretages stadig af dem selv.</aside><p class='muted'>Der sendes ingen e-mails. Ved offentlig adgang skal serveradressen bruge HTTPS.</p></section>");
    body.push_str(&format!("<section class='panel'><h2>Opret MCP-token</h2><form class='editor' method='post' action='/integrations/tokens'>{}<label>Navn<input name='name' required maxlength='100' placeholder='Fx Min AI-assistent' autocomplete='off'></label><label>Gyldighed<select name='days'><option value='7'>7 dage</option><option value='30'>30 dage</option><option value='90' selected>90 dage</option><option value='365'>365 dage</option></select></label><button>Opret token med fuld adminadgang</button></form></section>", csrf_field(secret)));
    body.push_str("<section class='panel'><h2>Dine MCP-tokens</h2><div class='table-wrap'><table><thead><tr><th>Navn</th><th>Status</th><th>Seneste brug</th><th>Udløber</th><th>Handling</th></tr></thead><tbody>");
    for t in data["tokens"].as_array().unwrap() {
        let status = t["status"].as_str().unwrap();
        let label = match status {
            "active" => "Aktivt",
            "expired" => "Udløbet",
            _ => "Tilbagekaldt",
        };
        let id = t["id"].as_i64().unwrap();
        body.push_str(&format!("<tr><td><strong>{}</strong><small class='muted'> #{id} · {}…</small></td><td>{label}</td><td>{}</td><td>{}</td><td>",esc(t["name"].as_str().unwrap()),esc(t["prefix"].as_str().unwrap()),t["last_used_at"].as_i64().map(db::date).unwrap_or_else(||"Ikke brugt".into()),db::date(t["expires_at"].as_i64().unwrap())));
        if status == "active" {
            body.push_str(&format!("<form method='post' action='/integrations/tokens/{id}/revoke'>{}<button class='danger small'>Tilbagekald</button></form>",csrf_field(secret)));
        }
        body.push_str("</td></tr>");
    }
    body.push_str("</tbody></table></div><p class='muted'>Hvert token tilbagekaldes separat, også tokens oprettet af din AI. Tilbagekaldelse afviser efterfølgende forespørgsler. Et igangværende kald kan allerede være udført. Adgangskodeskift med configure-admin tilbagekalder også alle MCP-tokens.</p></section><section class='panel'><h2>Seneste AI-aktivitet</h2><p class='muted'>De seneste 100 hændelser. Hemmelige tokens, links og dokumentindhold gemmes ikke i dette aktivitetsspor.</p><ol class='timeline'>");
    for e in data["activity"].as_array().unwrap() {
        let actor = e["name"].as_str().unwrap_or("Administrator");
        body.push_str(&format!(
            "<li><strong>{}</strong><small>{} · {}</small></li>",
            esc(e["detail"].as_str().unwrap()),
            esc(actor),
            db::date(e["at"].as_i64().unwrap())
        ));
    }
    body.push_str("</ol></section>");
    Ok(Html(page("AI / MCP", &body, Some(secret))))
}

#[cfg(test)]
mod proxy_tests {
    use super::*;
    fn config(trust_proxy: bool) -> Config {
        Config {
            base_url: "https://skrivunder.example".into(),
            bind: "127.0.0.1:3080".into(),
            data_dir: "var".into(),
            trust_proxy,
        }
    }
    #[test]
    fn proxy_client_ip_requires_explicit_trust_loopback_and_one_valid_header() {
        let local = "127.0.0.1:1234".parse().unwrap();
        let remote = "203.0.113.8:1234".parse().unwrap();
        let mut h = HeaderMap::new();
        h.insert("x-forwarded-for", HeaderValue::from_static("198.51.100.99"));
        h.insert(
            "x-skriv-under-client-ip",
            HeaderValue::from_static("198.51.100.7"),
        );
        assert_eq!(
            login_client_ip(&config(false), local, &h).ok(),
            Some("127.0.0.1".parse().unwrap())
        );
        assert_eq!(
            login_client_ip(&config(true), remote, &h).ok(),
            Some("203.0.113.8".parse().unwrap())
        );
        assert_eq!(
            login_client_ip(&config(true), local, &h).ok(),
            Some("198.51.100.7".parse().unwrap())
        );
        h.append(
            "x-skriv-under-client-ip",
            HeaderValue::from_static("198.51.100.8"),
        );
        assert!(login_client_ip(&config(true), local, &h).is_err());
        h.insert(
            "x-skriv-under-client-ip",
            HeaderValue::from_static("not-an-ip"),
        );
        assert!(login_client_ip(&config(true), local, &h).is_err());
        h.remove("x-skriv-under-client-ip");
        assert!(login_client_ip(&config(true), local, &h).is_err());
    }
}
