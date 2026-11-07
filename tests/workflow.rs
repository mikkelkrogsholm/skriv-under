use axum::{
    Router,
    body::Body,
    extract::ConnectInfo,
    http::{Request, StatusCode},
};
use document_signing::{
    config::Config,
    db, links, pdf,
    web::{self, App},
};
use http_body_util::BodyExt;
use lopdf::{Document, dictionary};
use sqlx::Row;
use std::{net::SocketAddr, sync::Arc};
use tower::ServiceExt;

struct TestApp {
    app: Arc<App>,
    router: Router,
    _dir: tempfile::TempDir,
    session: String,
}
struct Reply {
    status: StatusCode,
    body: Vec<u8>,
    cookie: Option<String>,
}
async fn request(
    router: &Router,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    body: Vec<u8>,
    content_type: &str,
) -> Reply {
    let mut builder = Request::builder()
        .method(method)
        .uri(path)
        .header("content-type", content_type);
    if let Some(c) = cookie {
        builder = builder.header("cookie", c);
    }
    let mut req = builder.body(Body::from(body)).unwrap();
    req.extensions_mut().insert(ConnectInfo(
        "127.0.0.1:54321".parse::<SocketAddr>().unwrap(),
    ));
    let r = router.clone().oneshot(req).await.unwrap();
    let status = r.status();
    let cookie = r
        .headers()
        .get("set-cookie")
        .map(|v| v.to_str().unwrap().split(';').next().unwrap().to_string());
    let body = r.into_body().collect().await.unwrap().to_bytes().to_vec();
    Reply {
        status,
        body,
        cookie,
    }
}
fn form(fields: &[(&str, &str)]) -> Vec<u8> {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields.iter().copied())
        .finish()
        .into_bytes()
}
fn sample_pdf() -> Vec<u8> {
    let mut d = Document::with_version("1.7");
    let pages = d.new_object_id();
    let page=d.add_object(dictionary!{"Type"=>"Page","Parent"=>pages,"MediaBox"=>vec![0.into(),0.into(),595.into(),842.into()]});
    d.objects.insert(
        pages,
        lopdf::Object::Dictionary(
            dictionary! {"Type"=>"Pages","Kids"=>vec![page.into()],"Count"=>1},
        ),
    );
    let catalog = d.add_object(dictionary! {"Type"=>"Catalog","Pages"=>pages});
    d.trailer.set("Root", catalog);
    let mut b = Vec::new();
    d.save_to(&mut b).unwrap();
    b
}
async fn setup() -> TestApp {
    let dir = tempfile::tempdir().unwrap();
    let config = Config {
        base_url: "http://127.0.0.1:3080".into(),
        bind: "127.0.0.1:0".into(),
        data_dir: dir.path().join("data"),
    };
    let pool = db::open(&config).await.unwrap();
    use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
    let password = Argon2::default()
        .hash_password(
            b"test-password-123",
            &SaltString::generate(&mut rand::rngs::OsRng),
        )
        .unwrap()
        .to_string();
    sqlx::query("INSERT INTO users(id,email,password_hash) VALUES(1,'owner@example.com',?)")
        .bind(&password)
        .execute(&pool)
        .await
        .unwrap();
    let session = db::token();
    sqlx::query("INSERT INTO sessions(token_hash,user_id,expires_at) VALUES(?,1,?)")
        .bind(db::hash(&session))
        .bind(db::now() + 3600)
        .execute(&pool)
        .await
        .unwrap();
    let link_key = links::load_key(&config.data_dir, &pool).await.unwrap();
    let app = Arc::new(App {
        pool,
        config,
        link_key,
    });
    let router = web::router(app.clone());
    TestApp {
        app,
        router,
        _dir: dir,
        session,
    }
}
impl TestApp {
    async fn post(&self, path: &str, fields: &[(&str, &str)]) -> Reply {
        let mut fields = fields.to_vec();
        let csrf = db::csrf(&self.session);
        fields.push(("csrf", &csrf));
        request(
            &self.router,
            "POST",
            path,
            Some(&format!("session={}", self.session)),
            form(&fields),
            "application/x-www-form-urlencoded",
        )
        .await
    }
    async fn upload(&self) -> i64 {
        let r = self.upload_form("Testaftale ÆØÅ", "Læs og bekræft").await;
        assert_eq!(
            r.status,
            StatusCode::SEE_OTHER,
            "{}",
            String::from_utf8_lossy(&r.body)
        );
        sqlx::query_scalar("SELECT max(id) FROM documents")
            .fetch_one(&self.app.pool)
            .await
            .unwrap()
    }
    async fn upload_form(&self, title: &str, message: &str) -> Reply {
        let boundary = "workflow-boundary";
        let mut b = Vec::new();
        for (key, value) in [
            ("csrf", db::csrf(&self.session)),
            ("title", title.to_string()),
            ("message", message.to_string()),
            ("recipients", "Anna Jensen\nBo Hansen".into()),
            ("days", "14".into()),
        ] {
            b.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{key}\"\r\n\r\n{value}\r\n").as_bytes());
        }
        b.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"pdf\"; filename=\"contract.pdf\"\r\nContent-Type: application/pdf\r\n\r\n").as_bytes());
        b.extend(sample_pdf());
        b.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        request(
            &self.router,
            "POST",
            "/documents",
            Some(&format!("session={}", self.session)),
            b,
            &format!("multipart/form-data; boundary={boundary}"),
        )
        .await
    }
    async fn tokens(&self, id: i64) -> Vec<String> {
        let response = request(
            &self.router,
            "GET",
            &format!("/documents/{id}"),
            Some(&format!("session={}", self.session)),
            vec![],
            "",
        )
        .await;
        assert_eq!(response.status, StatusCode::OK);
        String::from_utf8(response.body)
            .unwrap()
            .split("/s/")
            .skip(1)
            .map(|s| s[..64].to_string())
            .collect()
    }
    async fn sign(&self, token: &str) -> Reply {
        request(
            &self.router,
            "POST",
            "/sign",
            Some(&format!("signer={token}")),
            form(&[
                ("csrf", &db::csrf(token)),
                ("action", "sign"),
                ("name", "Anna ÆØÅ"),
                ("agree", "yes"),
            ]),
            "application/x-www-form-urlencoded",
        )
        .await
    }
    async fn state(&self, id: i64) -> String {
        sqlx::query_scalar("SELECT status FROM documents WHERE id=?")
            .bind(id)
            .fetch_one(&self.app.pool)
            .await
            .unwrap()
    }
}
#[tokio::test]
async fn complete_workflow_and_concurrent_replay() {
    let t = setup().await;
    let id = t.upload().await;
    assert_eq!(t.state(id).await, "draft");
    assert_eq!(
        t.post(&format!("/documents/{id}/start"), &[]).await.status,
        StatusCode::SEE_OTHER
    );
    let tokens = t.tokens(id).await;
    assert_eq!(tokens.len(), 2);
    let detail = request(
        &t.router,
        "GET",
        &format!("/documents/{id}"),
        Some(&format!("session={}", t.session)),
        vec![],
        "",
    )
    .await;
    assert!(String::from_utf8_lossy(&detail.body).contains(&tokens[0]));
    // Anonymous requests cannot access an owner's document.
    assert_eq!(
        request(
            &t.router,
            "GET",
            &format!("/documents/{id}/files/original"),
            None,
            vec![],
            ""
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    // A recipient token never grants access to the administrator portal.
    assert_eq!(
        request(
            &t.router,
            "GET",
            &format!("/documents/{id}"),
            Some(&format!("signer={}", tokens[0])),
            vec![],
            ""
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    for _ in 0..2 {
        assert_eq!(
            request(
                &t.router,
                "GET",
                &format!("/s/{}", tokens[0]),
                None,
                vec![],
                ""
            )
            .await
            .status,
            StatusCode::SEE_OTHER
        );
    }
    let signed: i64 = sqlx::query_scalar("SELECT count(*) FROM recipients WHERE status='signed'")
        .fetch_one(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(signed, 0);
    assert_eq!(
        request(
            &t.router,
            "POST",
            "/sign",
            Some(&format!("signer={}", tokens[0])),
            form(&[
                ("csrf", &db::csrf(&tokens[0])),
                ("action", "sign"),
                ("name", "Anna")
            ]),
            "application/x-www-form-urlencoded"
        )
        .await
        .status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        request(
            &t.router,
            "POST",
            "/sign",
            Some(&format!("signer={}", tokens[0])),
            form(&[
                ("csrf", "wrong"),
                ("action", "sign"),
                ("name", "Anna"),
                ("agree", "yes")
            ]),
            "application/x-www-form-urlencoded"
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(t.sign(&tokens[0]).await.status, StatusCode::SEE_OTHER);
    assert_eq!(t.state(id).await, "pending");
    assert_eq!(t.sign(&tokens[0]).await.status, StatusCode::BAD_REQUEST);
    let (a, b) = tokio::join!(t.sign(&tokens[1]), t.sign(&tokens[1]));
    assert!([a.status, b.status].contains(&StatusCode::SEE_OTHER));
    assert!([a.status, b.status].contains(&StatusCode::BAD_REQUEST));
    assert_eq!(t.state(id).await, "completed");
    let row = sqlx::query("SELECT pdf,final_pdf,receipt_pdf,final_hash FROM documents WHERE id=?")
        .bind(id)
        .fetch_one(&t.app.pool)
        .await
        .unwrap();
    let original: Vec<u8> = row.get("pdf");
    let final_pdf: Vec<u8> = row.get("final_pdf");
    let receipt: Vec<u8> = row.get("receipt_pdf");
    assert_eq!(db::hash(&final_pdf), row.get::<String, _>("final_hash"));
    assert_eq!(Document::load_mem(&final_pdf).unwrap().get_pages().len(), 2);
    assert_eq!(Document::load_mem(&receipt).unwrap().get_pages().len(), 1);
    assert_eq!(db::hash(original), db::hash(sample_pdf()));
    let detail = request(
        &t.router,
        "GET",
        &format!("/documents/{id}"),
        Some(&format!("session={}", t.session)),
        vec![],
        "",
    )
    .await;
    let body = String::from_utf8(detail.body).unwrap();
    let download = &body.split("/d/").nth(1).unwrap()[..64];
    assert_eq!(
        request(
            &t.router,
            "GET",
            &format!("/d/{download}/final"),
            None,
            vec![],
            ""
        )
        .await
        .body,
        final_pdf
    );
    sqlx::query("UPDATE downloads SET expires_at=0")
        .execute(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(
        request(
            &t.router,
            "GET",
            &format!("/d/{download}/final"),
            None,
            vec![],
            ""
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let mail_tables: i64 =
        sqlx::query_scalar("SELECT count(*) FROM sqlite_master WHERE name='mail_jobs'")
            .fetch_one(&t.app.pool)
            .await
            .unwrap();
    assert_eq!(mail_tables, 0);
    assert!(!t.app.config.data_dir.join("outbox").exists());
    let audit = request(
        &t.router,
        "GET",
        &format!("/documents/{id}/audit.json"),
        Some(&format!("session={}", t.session)),
        vec![],
        "",
    )
    .await;
    let json: serde_json::Value = serde_json::from_slice(&audit.body).unwrap();
    assert_eq!(json["document_reference"], format!("SU-{id}"));
    assert_eq!(json["signatures"].as_array().unwrap().len(), 2);
    for signature in json["signatures"].as_array().unwrap() {
        assert!(
            signature["signing_reference"]
                .as_str()
                .unwrap()
                .starts_with(&format!("SU-{id}-"))
        );
        assert_eq!(signature["consent"], db::CONSENT);
        assert!(signature["signed_at"].as_i64().unwrap() > 0);
    }

    assert!(
        json["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "signed" && e["detail"].as_str().unwrap().contains("ÆØÅ"))
    );
}
#[tokio::test]
async fn rotating_links_revokes_and_cancel_blocks_signing() {
    let t = setup().await;
    let id = t.upload().await;
    t.post(&format!("/documents/{id}/start"), &[]).await;
    let old = t.tokens(id).await;
    assert_eq!(
        t.post(&format!("/documents/{id}/rotate"), &[]).await.status,
        StatusCode::BAD_REQUEST
    );
    sqlx::query("UPDATE events SET at=at-61")
        .execute(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(
        t.post(&format!("/documents/{id}/rotate"), &[]).await.status,
        StatusCode::SEE_OTHER
    );
    let new = t.tokens(id).await;
    assert_ne!(old, new);
    assert_eq!(t.sign(&old[0]).await.status, StatusCode::FORBIDDEN);
    assert_eq!(
        t.post(&format!("/documents/{id}/cancel"), &[]).await.status,
        StatusCode::SEE_OTHER
    );
    assert_eq!(t.sign(&new[0]).await.status, StatusCode::BAD_REQUEST);
    assert_eq!(t.state(id).await, "cancelled");
}
#[tokio::test]
async fn expiry_and_decline_close_the_round() {
    let t = setup().await;
    let id = t.upload().await;
    t.post(&format!("/documents/{id}/start"), &[]).await;
    let tokens = t.tokens(id).await;
    sqlx::query("UPDATE documents SET expires_at=0 WHERE id=?")
        .bind(id)
        .execute(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(t.sign(&tokens[0]).await.status, StatusCode::BAD_REQUEST);
    db::expire(&t.app.pool).await.unwrap();
    assert_eq!(t.state(id).await, "expired");
    let id = t.upload().await;
    t.post(&format!("/documents/{id}/start"), &[]).await;
    let tokens = t.tokens(id).await;
    let r = request(
        &t.router,
        "POST",
        "/sign",
        Some(&format!("signer={}", tokens[0])),
        form(&[
            ("csrf", &db::csrf(&tokens[0])),
            ("action", "decline"),
            ("reason", "Forkert beløb"),
        ]),
        "application/x-www-form-urlencoded",
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(t.state(id).await, "declined");
    assert_eq!(t.sign(&tokens[1]).await.status, StatusCode::BAD_REQUEST);
    let jobs: i64 = sqlx::query_scalar("SELECT count(*) FROM events WHERE kind='declined'")
        .fetch_one(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(jobs, 1);
}
#[tokio::test]
async fn login_csrf_sessions_and_rate_limit() {
    let t = setup().await;
    let r = request(&t.router, "GET", "/login", None, vec![], "").await;
    let cookie = r.cookie.unwrap();
    let nonce = cookie.split_once('=').unwrap().1;
    let payload = form(&[
        ("csrf", &db::csrf(nonce)),
        ("email", "owner@example.com"),
        ("password", "test-password-123"),
    ]);
    let r = request(
        &t.router,
        "POST",
        "/login",
        Some(&cookie),
        payload,
        "application/x-www-form-urlencoded",
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let cookie = r.cookie.unwrap();
    assert_eq!(
        request(&t.router, "GET", "/", Some(&cookie), vec![], "")
            .await
            .status,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &t.router,
            "POST",
            "/logout",
            Some(&cookie),
            form(&[("csrf", "wrong")]),
            "application/x-www-form-urlencoded"
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    let secret = cookie.split_once('=').unwrap().1;
    assert_eq!(
        request(
            &t.router,
            "POST",
            "/logout",
            Some(&cookie),
            form(&[("csrf", &db::csrf(secret))]),
            "application/x-www-form-urlencoded"
        )
        .await
        .status,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        request(&t.router, "GET", "/", Some(&cookie), vec![], "")
            .await
            .status,
        StatusCode::SEE_OTHER
    );
    for _ in 0..10 {
        sqlx::query("INSERT INTO login_attempts(ip,at) VALUES('127.0.0.1',?)")
            .bind(db::now())
            .execute(&t.app.pool)
            .await
            .unwrap();
    }
    let cookie = format!("login_nonce={nonce}");
    let payload = form(&[
        ("csrf", &db::csrf(nonce)),
        ("email", "owner@example.com"),
        ("password", "test-password-123"),
    ]);
    assert_eq!(
        request(
            &t.router,
            "POST",
            "/login",
            Some(&cookie),
            payload,
            "application/x-www-form-urlencoded"
        )
        .await
        .status,
        StatusCode::TOO_MANY_REQUESTS
    );
}
#[test]
fn pdf_validation_rejects_active_and_invalid_documents() {
    assert!(pdf::validate(b"not a PDF").is_err());
    let mut doc = Document::load_mem(&sample_pdf()).unwrap();
    doc.add_object(dictionary! {"JS"=>"malicious"});
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    assert!(pdf::validate(&bytes).is_err());
}

#[tokio::test]
async fn only_one_administrator_can_exist() {
    let t = setup().await;
    let result =
        sqlx::query("INSERT INTO users(email,password_hash) VALUES('second@example.com','unused')")
            .execute(&t.app.pool)
            .await;
    assert!(result.is_err());
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[tokio::test]
async fn invitation_is_scoped_to_one_document() {
    let t = setup().await;
    let first = t.upload().await;
    let second = t.upload().await;
    t.post(&format!("/documents/{first}/start"), &[]).await;
    t.post(&format!("/documents/{second}/start"), &[]).await;
    let tokens = t.tokens(first).await;
    for token in tokens {
        assert_eq!(t.sign(&token).await.status, StatusCode::SEE_OTHER);
    }
    assert_eq!(t.state(first).await, "completed");
    assert_eq!(t.state(second).await, "pending");
    let signed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM recipients WHERE document_id=? AND status='signed'",
    )
    .bind(second)
    .fetch_one(&t.app.pool)
    .await
    .unwrap();
    assert_eq!(signed, 0);
}

#[tokio::test]
async fn process_overview_tracks_recipients_without_exposing_admin_access() {
    let t = setup().await;
    let id = t.upload().await;
    t.post(&format!("/documents/{id}/start"), &[]).await;
    let tokens = t.tokens(id).await;
    let admin = format!("session={}", t.session);
    let recipient = format!("signer={}", tokens[0]);
    let before = request(
        &t.router,
        "GET",
        &format!("/documents/{id}/status"),
        Some(&admin),
        vec![],
        "",
    )
    .await;
    let before: serde_json::Value = serde_json::from_slice(&before.body).unwrap();
    request(
        &t.router,
        "GET",
        &format!("/s/{}", tokens[0]),
        None,
        vec![],
        "",
    )
    .await;
    request(
        &t.router,
        "GET",
        "/sign/document.pdf",
        Some(&recipient),
        vec![],
        "",
    )
    .await;
    assert_eq!(t.sign(&tokens[0]).await.status, StatusCode::SEE_OTHER);
    let status = request(
        &t.router,
        "GET",
        &format!("/documents/{id}/status"),
        Some(&admin),
        vec![],
        "",
    )
    .await;
    let status: serde_json::Value = serde_json::from_slice(&status.body).unwrap();
    assert_eq!(status["status"], "pending");
    assert!(status["revision"].as_i64().unwrap() > before["revision"].as_i64().unwrap());
    let detail = request(
        &t.router,
        "GET",
        &format!("/documents/{id}"),
        Some(&admin),
        vec![],
        "",
    )
    .await;
    let html = String::from_utf8(detail.body).unwrap();
    for text in [
        "1 af 2 underskrevet",
        "Afventer: Bo Hansen",
        "Link åbnet (kan være automatisk)",
        "PDF tilgået (ikke læsebevis)",
        "Personligt signeringslink til Bo Hansen",
        "Underskrift registreret",
        "data-process-id",
    ] {
        assert!(html.contains(text), "Missing {text}");
    }
    assert!(!html.contains(&tokens[0]));
    let list = request(
        &t.router,
        "GET",
        "/?status=pending",
        Some(&admin),
        vec![],
        "",
    )
    .await;
    assert!(
        String::from_utf8(list.body)
            .unwrap()
            .contains("1 af 2 underskrevet")
    );
    for path in [
        format!("/documents/{id}/status"),
        format!("/documents/{id}/audit.json"),
        format!("/documents/{id}/files/final"),
    ] {
        assert_eq!(
            request(&t.router, "GET", &path, Some(&recipient), vec![], "")
                .await
                .status,
            StatusCode::FORBIDDEN
        );
    }
}

#[tokio::test]
async fn signed_archive_survives_download_expiry_and_restart() {
    let t = setup().await;
    let id = t.upload().await;
    let draft = t.upload().await;
    sqlx::query("UPDATE documents SET title='Anden kladde' WHERE id=?")
        .bind(draft)
        .execute(&t.app.pool)
        .await
        .unwrap();
    t.post(&format!("/documents/{id}/start"), &[]).await;
    let tokens = t.tokens(id).await;
    for token in &tokens {
        assert_eq!(t.sign(token).await.status, StatusCode::SEE_OTHER);
    }
    sqlx::query("UPDATE downloads SET expires_at=0")
        .execute(&t.app.pool)
        .await
        .unwrap();
    let expected: Vec<u8> = sqlx::query_scalar("SELECT final_pdf FROM documents WHERE id=?")
        .bind(id)
        .fetch_one(&t.app.pool)
        .await
        .unwrap();
    // Reopen the actual SQLite file to verify durable access rather than cached state.
    t.app.pool.close().await;
    let pool = db::open(&t.app.config).await.unwrap();
    let link_key = links::load_key(&t.app.config.data_dir, &pool)
        .await
        .unwrap();
    let router = web::router(Arc::new(App {
        pool,
        config: t.app.config.clone(),
        link_key,
    }));
    let admin = format!("session={}", t.session);
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("q", "æøå")
        .finish();
    let archive = request(
        &router,
        "GET",
        &format!("/archive?{query}"),
        Some(&admin),
        vec![],
        "",
    )
    .await;
    assert_eq!(archive.status, StatusCode::OK);
    let html = String::from_utf8(archive.body).unwrap();
    assert!(html.contains("Testaftale ÆØÅ"));
    assert!(html.contains("2 af 2 underskrevet"));
    assert!(!html.contains("Anden kladde"));
    assert!(html.contains(&format!("/documents/{id}/files/final")));
    assert!(html.contains(&format!("/documents/{id}/files/receipt")));
    assert!(html.contains(&format!("/documents/{id}")));
    assert_eq!(
        request(
            &router,
            "GET",
            &format!("/documents/{id}/files/final"),
            Some(&admin),
            vec![],
            ""
        )
        .await
        .body,
        expected
    );
    assert_eq!(
        request(
            &router,
            "GET",
            &format!("/documents/{id}/files/receipt"),
            Some(&admin),
            vec![],
            ""
        )
        .await
        .status,
        StatusCode::OK
    );
    let no_matches = request(
        &router,
        "GET",
        "/archive?q=ingen-resultater",
        Some(&admin),
        vec![],
        "",
    )
    .await;
    assert!(
        !String::from_utf8(no_matches.body)
            .unwrap()
            .contains("Testaftale ÆØÅ")
    );
    let name_search = request(&router, "GET", "/archive?q=Anna", Some(&admin), vec![], "").await;
    assert!(
        String::from_utf8(name_search.body)
            .unwrap()
            .contains("Testaftale ÆØÅ")
    );
    let hidden = request(
        &router,
        "GET",
        "/archive",
        Some(&format!("signer={}", tokens[0])),
        vec![],
        "",
    )
    .await;
    assert_eq!(hidden.status, StatusCode::SEE_OTHER);
    assert!(
        !String::from_utf8(hidden.body)
            .unwrap()
            .contains("Testaftale ÆØÅ")
    );
}

#[tokio::test]
async fn links_can_be_recopied_after_restart_and_download_links_rotated() {
    let mut t = setup().await;
    let id = t.upload().await;
    t.post(&format!("/documents/{id}/start"), &[]).await;
    let original = t.tokens(id).await;
    assert_eq!(original, t.tokens(id).await);
    let legacy_emails: i64 =
        sqlx::query_scalar("SELECT count(*) FROM recipients WHERE email IS NOT NULL")
            .fetch_one(&t.app.pool)
            .await
            .unwrap();
    assert_eq!(legacy_emails, 0);
    t.app.pool.close().await;
    let pool = db::open(&t.app.config).await.unwrap();
    let link_key = links::load_key(&t.app.config.data_dir, &pool)
        .await
        .unwrap();
    t.app = Arc::new(App {
        pool,
        config: t.app.config.clone(),
        link_key,
    });
    t.router = web::router(t.app.clone());
    assert_eq!(original, t.tokens(id).await);
    let no_admin = request(
        &t.router,
        "GET",
        &format!("/documents/{id}"),
        Some(&format!("signer={}", original[0])),
        vec![],
        "",
    )
    .await;
    assert_eq!(no_admin.status, StatusCode::FORBIDDEN);
    assert!(
        !String::from_utf8(no_admin.body)
            .unwrap()
            .contains(&original[1])
    );
    for token in &original {
        assert_eq!(t.sign(token).await.status, StatusCode::SEE_OTHER);
    }
    let admin = format!("session={}", t.session);
    let detail = request(
        &t.router,
        "GET",
        &format!("/documents/{id}"),
        Some(&admin),
        vec![],
        "",
    )
    .await;
    let html = String::from_utf8(detail.body).unwrap();
    let old = html.split("/d/").nth(1).unwrap()[..64].to_string();
    assert_eq!(
        t.post(&format!("/documents/{id}/renew-download"), &[])
            .await
            .status,
        StatusCode::SEE_OTHER
    );
    let detail = request(
        &t.router,
        "GET",
        &format!("/documents/{id}"),
        Some(&admin),
        vec![],
        "",
    )
    .await;
    let html = String::from_utf8(detail.body).unwrap();
    let new = &html.split("/d/").nth(1).unwrap()[..64];
    assert_ne!(old, new);
    assert_eq!(
        request(
            &t.router,
            "GET",
            &format!("/d/{old}/final"),
            None,
            vec![],
            ""
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        request(
            &t.router,
            "GET",
            &format!("/d/{new}/final"),
            None,
            vec![],
            ""
        )
        .await
        .status,
        StatusCode::OK
    );
    assert_eq!(
        request(
            &t.router,
            "GET",
            &format!("/d/{new}/original"),
            None,
            vec![],
            ""
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
}

#[tokio::test]
async fn missing_key_does_not_silently_invalidate_existing_links() {
    let t = setup().await;
    let id = t.upload().await;
    t.post(&format!("/documents/{id}/start"), &[]).await;
    let path = t.app.config.data_dir.join("link-key");
    let key = std::fs::read(&path).unwrap();
    std::fs::remove_file(&path).unwrap();
    assert!(
        links::load_key(&t.app.config.data_dir, &t.app.pool)
            .await
            .is_err()
    );
    assert!(!path.exists());
    std::fs::write(&path, key).unwrap();
    assert_eq!(
        links::load_key(&t.app.config.data_dir, &t.app.pool)
            .await
            .unwrap(),
        t.app.link_key
    );
}

#[tokio::test]
async fn upgrades_legacy_database_preserving_signed_documents_and_disabling_mail() {
    let mut t = setup().await;
    let id = t.upload().await;
    t.post(&format!("/documents/{id}/start"), &[]).await;
    for token in t.tokens(id).await {
        assert_eq!(t.sign(&token).await.status, StatusCode::SEE_OTHER);
    }
    let expected: Vec<u8> = sqlx::query_scalar("SELECT final_pdf FROM documents WHERE id=?")
        .bind(id)
        .fetch_one(&t.app.pool)
        .await
        .unwrap();
    // Reproduce the previous required-email schema and an unsent legacy mail job.
    sqlx::raw_sql("PRAGMA foreign_keys=OFF;
        BEGIN;
        CREATE TABLE legacy_recipients (
          id INTEGER PRIMARY KEY, document_id INTEGER NOT NULL REFERENCES documents(id), name TEXT NOT NULL,
          email TEXT NOT NULL, token_hash TEXT UNIQUE, status TEXT NOT NULL DEFAULT 'pending',
          signed_name TEXT,signed_at INTEGER,reason TEXT,consent TEXT,UNIQUE(document_id,email));
        INSERT INTO legacy_recipients SELECT id,document_id,name,'legacy-'||id||'@example.invalid',token_hash,status,signed_name,signed_at,reason,consent FROM recipients;
        DROP TABLE recipients;
        ALTER TABLE legacy_recipients RENAME TO recipients;
        CREATE TABLE mail_jobs(id INTEGER PRIMARY KEY,state TEXT,body TEXT);
        INSERT INTO mail_jobs VALUES(1,'pending','legacy secret invitation');
        COMMIT;
        PRAGMA foreign_keys=ON;")
        .execute(&t.app.pool).await.unwrap();
    t.app.pool.close().await;
    let pool = db::open(&t.app.config).await.unwrap();
    let link_key = links::load_key(&t.app.config.data_dir, &pool)
        .await
        .unwrap();
    assert!(
        sqlx::query("PRAGMA foreign_key_check")
            .fetch_all(&pool)
            .await
            .unwrap()
            .is_empty()
    );
    let row = sqlx::query("SELECT state,body FROM mail_jobs WHERE id=1")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(row.get::<String, _>("state"), "cancelled");
    assert_eq!(row.get::<String, _>("body"), "");
    let actual: Vec<u8> = sqlx::query_scalar("SELECT final_pdf FROM documents WHERE id=?")
        .bind(id)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(actual, expected);
    let signed: i64 = sqlx::query_scalar(
        "SELECT count(*) FROM recipients WHERE document_id=? AND status='signed'",
    )
    .bind(id)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(signed, 2);
    t.app = Arc::new(App {
        pool,
        config: t.app.config.clone(),
        link_key,
    });
    t.router = web::router(t.app.clone());
    let new_id = t.upload().await;
    assert!(new_id > id);
    assert_eq!(
        t.post(&format!("/documents/{new_id}/start"), &[])
            .await
            .status,
        StatusCode::SEE_OTHER
    );
    assert_eq!(t.tokens(new_id).await.len(), 2);
    // The migration is safe to run again on each startup.
    t.app.pool.close().await;
    assert!(db::open(&t.app.config).await.is_ok());
}

fn receipt_signer(id: i64) -> db::Recipient {
    db::Recipient {
        id,
        document_id: 42,
        name: format!("Modtager {id}"),
        email: None,
        status: "signed".into(),
        signed_name: Some(format!("Underskriver {id} ÆØÅ")),
        signed_at: Some(1_700_000_000 + id),
        reason: None,
        consent: Some("Jeg accepterer version 7 af aftalen.".into()),
    }
}
#[test]
fn signature_appendix_contains_actual_consent_names_references_and_precise_times() {
    let original = sample_pdf();
    let hash = db::hash(&original);
    let people = vec![receipt_signer(1), receipt_signer(2)];
    let (signed, receipt) = pdf::complete(&original, 42, "Prøveaftale", &hash, &people).unwrap();
    let receipt = Document::load_mem(&receipt).unwrap();
    assert_eq!(receipt.get_pages().len(), 1);
    let text = receipt.extract_text(&[1]).unwrap();
    for expected in [
        "Underskrifter og dokumentation",
        "SU-42-1",
        "SU-42-2",
        "Underskriver 1 ÆØÅ",
        "Underskriver 2 ÆØÅ",
        "2023-11-14 22:13:21 UTC",
        "Jeg accepterer version 7 af aftalen.",
        &hash,
        "1 af 1",
    ] {
        assert!(text.contains(expected), "Missing {expected}: {text}");
    }
    assert!(!text.contains(db::CONSENT));
    assert!(!text.contains("/s/") && !text.contains("/d/"));
    let signed = Document::load_mem(&signed).unwrap();
    assert_eq!(signed.get_pages().len(), 2);
    assert_eq!(signed.extract_text(&[2]).unwrap(), text);
}
#[test]
fn signature_appendix_paginates_all_signers_and_overrides_inherited_crop_and_rotation() {
    let mut source = Document::load_mem(&sample_pdf()).unwrap();
    let root = source
        .catalog()
        .unwrap()
        .get(b"Pages")
        .unwrap()
        .as_reference()
        .unwrap();
    let dict = source.get_object_mut(root).unwrap().as_dict_mut().unwrap();
    dict.set("Rotate", 90);
    dict.set("CropBox", vec![0.into(), 0.into(), 100.into(), 100.into()]);
    let mut original = Vec::new();
    source.save_to(&mut original).unwrap();
    let people = (1..=20)
        .map(|i| {
            let mut r = receipt_signer(i);
            r.signed_name = Some(format!("{} {i}", "W".repeat(115)));
            r
        })
        .collect::<Vec<_>>();
    let (signed, receipt) = pdf::complete(
        &original,
        42,
        &"W".repeat(150),
        &db::hash(&original),
        &people,
    )
    .unwrap();
    let signed = Document::load_mem(&signed).unwrap();
    let receipt = Document::load_mem(&receipt).unwrap();
    let pages = receipt.get_pages().keys().copied().collect::<Vec<_>>();
    assert!(pages.len() > 1);
    assert_eq!(signed.get_pages().len(), pages.len() + 1);
    let text = receipt.extract_text(&pages).unwrap();
    let normalized = text
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect::<String>();
    for r in people {
        assert!(normalized.contains(&format!("SU-42-{}", r.id)));
        assert!(normalized.contains(&r.signed_name.unwrap().replace(' ', "")));
    }
    for number in 2..=signed.get_pages().len() as u32 {
        let page = signed
            .get_object(signed.get_pages()[&number])
            .unwrap()
            .as_dict()
            .unwrap();
        assert_eq!(page.get(b"Rotate").unwrap().as_i64().unwrap(), 0);
        assert_eq!(
            page.get(b"CropBox").unwrap().as_array().unwrap()[2]
                .as_i64()
                .unwrap(),
            595
        );
        let content =
            lopdf::content::Content::decode(&signed.get_page_content(signed.get_pages()[&number]))
                .unwrap();
        for op in content.operations.iter().filter(|op| op.operator == "Tm") {
            let y = op.operands[5].as_i64().unwrap();
            assert!((40..=797).contains(&y));
        }
    }
}
#[test]
fn signature_appendix_rejects_incomplete_or_mismatched_evidence() {
    let original = sample_pdf();
    let hash = db::hash(&original);
    let mut r = receipt_signer(1);
    r.consent = None;
    assert!(pdf::complete(&original, 42, "Test", &hash, &[r.clone()]).is_err());
    r.consent = Some("Accepteret".into());
    r.signed_at = None;
    assert!(pdf::complete(&original, 42, "Test", &hash, &[r.clone()]).is_err());
    r.signed_at = Some(1_700_000_001);
    r.document_id = 43;
    assert!(pdf::complete(&original, 42, "Test", &hash, &[r.clone()]).is_err());
    r.document_id = 42;
    assert!(pdf::complete(&original, 42, "Test", "wrong-hash", &[r]).is_err());
}

#[tokio::test]
async fn wrong_restored_key_is_rejected_for_signing_and_completed_read_links() {
    let t = setup().await;
    let id = t.upload().await;
    t.post(&format!("/documents/{id}/start"), &[]).await;
    for token in t.tokens(id).await {
        assert_eq!(t.sign(&token).await.status, StatusCode::SEE_OTHER);
    }
    let path = t.app.config.data_dir.join("link-key");
    let correct = std::fs::read(&path).unwrap();
    for download_only in [false, true] {
        if download_only {
            sqlx::query("DELETE FROM recipient_links")
                .execute(&t.app.pool)
                .await
                .unwrap();
        }
        std::fs::write(&path, [0u8; 32]).unwrap();
        assert!(
            links::load_key(&t.app.config.data_dir, &t.app.pool)
                .await
                .is_err()
        );
        std::fs::write(&path, &correct).unwrap();
        assert_eq!(
            links::load_key(&t.app.config.data_dir, &t.app.pool)
                .await
                .unwrap(),
            t.app.link_key
        );
    }
}

#[test]
fn credential_change_revokes_a_login_already_waiting_for_password_verification() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .unwrap();
    runtime.block_on(async {
        let t = setup().await;
        let response = request(&t.router, "GET", "/login", None, vec![], "").await;
        let cookie = response.cookie.unwrap();
        let nonce = cookie.split_once('=').unwrap().1;
        let payload = form(&[
            ("csrf", &db::csrf(nonce)),
            ("email", "owner@example.com"),
            ("password", "test-password-123"),
        ]);
        // A deterministic barrier pauses password verification after the login has read
        // the old credentials, while allowing configure-admin's database work to finish.
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (release_tx, release_rx) = std::sync::mpsc::channel();
        let blocker = tokio::task::spawn_blocking(move || {
            started_tx.send(()).unwrap();
            release_rx
                .recv_timeout(std::time::Duration::from_secs(5))
                .unwrap();
        });
        started_rx.await.unwrap();
        let router = t.router.clone();
        let login = tokio::spawn(async move {
            request(
                &router,
                "POST",
                "/login",
                Some(&cookie),
                payload,
                "application/x-www-form-urlencoded",
            )
            .await
        });
        tokio::time::timeout(std::time::Duration::from_secs(2), async {
            loop {
                let attempts: i64 = sqlx::query_scalar("SELECT count(*) FROM login_attempts")
                    .fetch_one(&t.app.pool)
                    .await
                    .unwrap();
                if attempts > 0 {
                    break;
                }
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
        let updated = Argon2::default()
            .hash_password(
                b"new-password-456",
                &SaltString::generate(&mut rand::rngs::OsRng),
            )
            .unwrap()
            .to_string();
        let mut tx = t.app.pool.begin().await.unwrap();
        sqlx::query("UPDATE users SET password_hash=?")
            .bind(updated)
            .execute(&mut *tx)
            .await
            .unwrap();
        sqlx::query("DELETE FROM sessions")
            .execute(&mut *tx)
            .await
            .unwrap();
        tx.commit().await.unwrap();
        release_tx.send(()).unwrap();
        blocker.await.unwrap();
        assert_eq!(login.await.unwrap().status, StatusCode::UNAUTHORIZED);
        let sessions: i64 = sqlx::query_scalar("SELECT count(*) FROM sessions")
            .fetch_one(&t.app.pool)
            .await
            .unwrap();
        assert_eq!(sessions, 0);
    });
}

#[tokio::test]
async fn field_limits_count_characters_for_unicode_titles_messages_and_decline_reasons() {
    let t = setup().await;
    assert_eq!(
        t.upload_form(&"界".repeat(150), &"界".repeat(4000))
            .await
            .status,
        StatusCode::SEE_OTHER
    );
    assert_eq!(
        t.upload_form(&"界".repeat(151), "").await.status,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        t.upload_form("Test", &"界".repeat(4001)).await.status,
        StatusCode::BAD_REQUEST
    );
    let id = t.upload().await;
    t.post(&format!("/documents/{id}/start"), &[]).await;
    let tokens = t.tokens(id).await;
    let reason = "界".repeat(1000);
    let reply = request(
        &t.router,
        "POST",
        "/sign",
        Some(&format!("signer={}", tokens[0])),
        form(&[
            ("csrf", &db::csrf(&tokens[0])),
            ("action", "decline"),
            ("reason", &reason),
        ]),
        "application/x-www-form-urlencoded",
    )
    .await;
    assert_eq!(reply.status, StatusCode::SEE_OTHER);
    let stored: String = sqlx::query_scalar(
        "SELECT reason FROM recipients WHERE document_id=? AND status='declined'",
    )
    .bind(id)
    .fetch_one(&t.app.pool)
    .await
    .unwrap();
    assert_eq!(stored, reason);
}

#[test]
fn help_version_and_unknown_commands_never_start_server_or_create_data() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("not-created");
    for arg in ["--help", "--version", "unknown-command"] {
        let output = std::process::Command::new(env!("CARGO_BIN_EXE_document-signing"))
            .arg(arg)
            .env("DATA_DIR", &data)
            .env("BASE_URL", "invalid-origin")
            .output()
            .unwrap();
        assert_eq!(output.status.success(), arg != "unknown-command");
        assert!(!data.exists());
    }
}

// The same admin actions are reachable through HTML and bearer-authenticated MCP.
async fn mcp_request(
    t: &TestApp,
    token: Option<&str>,
    method: &str,
    body: serde_json::Value,
    extra: &[(&str, &str)],
) -> (StatusCode, serde_json::Value) {
    let mut builder = Request::builder()
        .method(method)
        .uri("/mcp")
        .header("host", "127.0.0.1:3080")
        .header("content-type", "application/json")
        .header("accept", "application/json, text/event-stream")
        .header("mcp-protocol-version", "2025-11-25");
    if let Some(token) = token {
        builder = builder.header("authorization", format!("Bearer {token}"));
    }
    for (k, v) in extra {
        builder.headers_mut().unwrap().insert(
            axum::http::HeaderName::from_bytes(k.as_bytes()).unwrap(),
            axum::http::HeaderValue::from_str(v).unwrap(),
        );
    }
    let response = t
        .router
        .clone()
        .oneshot(builder.body(Body::from(body.to_string())).unwrap())
        .await
        .unwrap();
    let status = response.status();
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    (
        status,
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
    )
}
async fn mcp_call(
    t: &TestApp,
    token: &str,
    name: &str,
    args: serde_json::Value,
) -> serde_json::Value {
    let (status,response)=mcp_request(t,Some(token),"POST",serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":args}}),&[]).await;
    assert_eq!(status, StatusCode::OK);
    assert!(response.get("error").is_none(), "{response}");
    response["result"].clone()
}
async fn mcp_token(t: &TestApp) -> String {
    document_signing::mcp::create_token(
        &t.app,
        1,
        "Integrationstest",
        90,
        document_signing::mcp::Issuer::Session(&t.session),
    )
    .await
    .unwrap_or_else(|_| panic!("Token creation failed"))["token"]
        .as_str()
        .unwrap()
        .to_string()
}
#[tokio::test]
async fn mcp_token_ui_requires_admin_csrf_shows_secret_once_and_revokes_independently() {
    let t = setup().await;
    for cookie in [None, Some("signer=invalid")] {
        assert_eq!(
            request(&t.router, "GET", "/integrations", cookie, vec![], "")
                .await
                .status,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            request(
                &t.router,
                "POST",
                "/integrations/tokens",
                cookie,
                form(&[("name", "AI"), ("days", "90"), ("csrf", "invalid")]),
                "application/x-www-form-urlencoded"
            )
            .await
            .status,
            StatusCode::FORBIDDEN
        );
    }
    let r = request(
        &t.router,
        "POST",
        "/integrations/tokens",
        Some(&format!("session={}", t.session)),
        form(&[("name", "AI"), ("days", "90"), ("csrf", "invalid")]),
        "application/x-www-form-urlencoded",
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = t
        .post(
            "/integrations/tokens",
            &[("name", "<AI-test>"), ("days", "90")],
        )
        .await;
    assert_eq!(r.status, StatusCode::OK);
    let html = String::from_utf8(r.body).unwrap();
    assert!(html.contains("&lt;AI-test&gt;"));
    let token = html.split("value='su_mcp_").nth(1).unwrap()[..64].to_string();
    let token = format!("su_mcp_{token}");
    let stored: (i64, String, String) =
        sqlx::query_as("SELECT id,token_hash,prefix FROM api_tokens")
            .fetch_one(&t.app.pool)
            .await
            .unwrap();
    assert_eq!(stored.1, db::hash(&token));
    assert_ne!(stored.1, token);
    assert_eq!(stored.2, token[..15]);
    let r = request(
        &t.router,
        "GET",
        "/integrations",
        Some(&format!("session={}", t.session)),
        vec![],
        "",
    )
    .await;
    assert!(!String::from_utf8(r.body).unwrap().contains(&token));
    let other = mcp_token(&t).await;
    assert_eq!(
        t.post(&format!("/integrations/tokens/{}/revoke", stored.0), &[])
            .await
            .status,
        StatusCode::SEE_OTHER
    );
    let ping = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"ping"});
    assert_eq!(
        mcp_request(&t, Some(&token), "POST", ping.clone(), &[])
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        mcp_request(&t, Some(&other), "POST", ping.clone(), &[])
            .await
            .0,
        StatusCode::OK
    );
    sqlx::query("UPDATE api_tokens SET expires_at=? WHERE token_hash=?")
        .bind(db::now() - 1)
        .bind(db::hash(&other))
        .execute(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(
        mcp_request(&t, Some(&other), "POST", ping, &[]).await.0,
        StatusCode::UNAUTHORIZED
    );
}
#[tokio::test]
async fn mcp_protocol_authentication_origin_and_notifications_cannot_bypass_permissions() {
    let t = setup().await;
    let token = mcp_token(&t).await;
    let ping = serde_json::json!({"jsonrpc":"2.0","id":"ping","method":"ping"});
    for wrong in [None, Some(t.session.as_str()), Some("su_mcp_wrong")] {
        assert_eq!(
            mcp_request(
                &t,
                wrong,
                "POST",
                ping.clone(),
                &[("cookie", &format!("session={}", t.session))]
            )
            .await
            .0,
            StatusCode::UNAUTHORIZED
        );
    }
    for header in [
        ("origin", "https://evil.example"),
        ("origin", "null"),
        ("host", "evil.example"),
    ] {
        assert_eq!(
            mcp_request(&t, Some(&token), "POST", ping.clone(), &[header])
                .await
                .0,
            StatusCode::FORBIDDEN
        );
    }
    assert_eq!(
        mcp_request(
            &t,
            Some(&token),
            "POST",
            ping.clone(),
            &[("origin", "http://127.0.0.1:3080")]
        )
        .await
        .0,
        StatusCode::OK
    );
    assert_eq!(
        mcp_request(
            &t,
            Some(&token),
            "POST",
            ping.clone(),
            &[("mcp-protocol-version", "invalid")]
        )
        .await
        .0,
        StatusCode::BAD_REQUEST
    );
    assert_eq!(
        mcp_request(
            &t,
            Some(&token),
            "POST",
            ping.clone(),
            &[("accept", "text/html")]
        )
        .await
        .0,
        StatusCode::NOT_ACCEPTABLE
    );
    assert_eq!(
        mcp_request(&t, Some(&token), "GET", serde_json::json!({}), &[])
            .await
            .0,
        StatusCode::METHOD_NOT_ALLOWED
    );
    let init = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","clientInfo":{"name":"test","version":"1"},"capabilities":{}}});
    let r = mcp_request(&t, Some(&token), "POST", init, &[]).await;
    assert_eq!(r.1["result"]["protocolVersion"], "2025-11-25");
    let r = mcp_request(
        &t,
        Some(&token),
        "POST",
        serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
        &[],
    )
    .await;
    assert_eq!(r.0, StatusCode::ACCEPTED);
    assert!(r.1.is_null());
    let r=mcp_request(&t,Some(&token),"POST",serde_json::json!({"jsonrpc":"2.0","method":"tools/call","params":{"name":"create_admin_token","arguments":{"name":"Hidden","days":90}}}),&[]).await;
    assert_eq!(r.0, StatusCode::BAD_REQUEST);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM api_tokens")
        .fetch_one(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let r = mcp_request(
        &t,
        Some(&token),
        "POST",
        serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/list"}),
        &[],
    )
    .await;
    let tools = r.1["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 12);
    let names = tools
        .iter()
        .map(|v| v["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    for name in [
        "list_documents",
        "get_document",
        "create_document",
        "start_signing",
        "rotate_signing_links",
        "cancel_document",
        "renew_download_links",
        "get_document_file",
        "get_audit_trail",
        "list_admin_tokens",
        "create_admin_token",
        "revoke_admin_token",
    ] {
        assert!(names.contains(&name));
    }
    assert_eq!(
        mcp_call(
            &t,
            &token,
            "get_document",
            serde_json::json!({"document_id":9999})
        )
        .await["isError"],
        true
    );
    assert_eq!(
        mcp_call(
            &t,
            &token,
            "get_document",
            serde_json::json!({"document_id":1,"ignored":true})
        )
        .await["isError"],
        true
    );
    let r=mcp_request(&t,Some(&token),"POST",serde_json::json!({"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"sign_document","arguments":{}}}),&[]).await;
    assert_eq!(r.1["error"]["code"], -32602);
}
#[tokio::test]
async fn mcp_has_document_workflow_parity_with_html_and_preserves_recipient_consent() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use serde_json::json;
    let t = setup().await;
    let token = mcp_token(&t).await;
    let pdf = sample_pdf();
    let created=mcp_call(&t,&token,"create_document",json!({"title":"MCP prøveaftale ÆØÅ","message":"Læs og bekræft","days":14,"recipients":["Anna Jensen","Bo Hansen"],"pdf_base64":STANDARD.encode(&pdf)})).await;
    assert_eq!(created["isError"], false);
    let doc = &created["structuredContent"];
    let id = doc["id"].as_i64().unwrap();
    assert_eq!(doc["status"], "draft");
    assert!(doc["recipients"][0]["signing_url"].is_null());
    let html = request(
        &t.router,
        "GET",
        &format!("/documents/{id}"),
        Some(&format!("session={}", t.session)),
        vec![],
        "",
    )
    .await;
    assert!(
        String::from_utf8(html.body)
            .unwrap()
            .contains("MCP prøveaftale ÆØÅ")
    );
    let listed = mcp_call(
        &t,
        &token,
        "list_documents",
        json!({"q":"bo hansen","status":"draft"}),
    )
    .await;
    assert_eq!(listed["structuredContent"]["total"], 1);
    let original = mcp_call(
        &t,
        &token,
        "get_document_file",
        json!({"document_id":id,"kind":"original"}),
    )
    .await;
    assert_eq!(
        STANDARD
            .decode(original["content"][1]["resource"]["blob"].as_str().unwrap())
            .unwrap(),
        pdf
    );
    let started = mcp_call(&t, &token, "start_signing", json!({"document_id":id})).await;
    assert_eq!(started["structuredContent"]["status"], "pending");
    let old = t.tokens(id).await;
    assert_eq!(old.len(), 2);
    assert_eq!(
        mcp_call(
            &t,
            &token,
            "rotate_signing_links",
            json!({"document_id":id})
        )
        .await["isError"],
        true
    );
    assert_eq!(t.sign(&old[0]).await.status, StatusCode::SEE_OTHER);
    sqlx::query("UPDATE events SET at=at-61 WHERE document_id=? AND kind='link_created'")
        .bind(id)
        .execute(&t.app.pool)
        .await
        .unwrap();
    let rotated = mcp_call(
        &t,
        &token,
        "rotate_signing_links",
        json!({"document_id":id}),
    )
    .await;
    assert_eq!(rotated["isError"], false);
    assert_eq!(t.sign(&old[1]).await.status, StatusCode::FORBIDDEN);
    let pending = t.tokens(id).await;
    assert_eq!(pending.len(), 1);
    assert_eq!(t.sign(&pending[0]).await.status, StatusCode::SEE_OTHER);
    assert_eq!(t.state(id).await, "completed");
    let archive = mcp_call(
        &t,
        &token,
        "list_documents",
        json!({"archive":true,"q":"MCP"}),
    )
    .await;
    assert_eq!(archive["structuredContent"]["total"], 1);
    for kind in ["final", "receipt"] {
        let r = mcp_call(
            &t,
            &token,
            "get_document_file",
            json!({"document_id":id,"kind":kind}),
        )
        .await;
        let bytes = STANDARD
            .decode(r["content"][1]["resource"]["blob"].as_str().unwrap())
            .unwrap();
        assert!(bytes.starts_with(b"%PDF-"));
        let html = request(
            &t.router,
            "GET",
            &format!("/documents/{id}/files/{kind}"),
            Some(&format!("session={}", t.session)),
            vec![],
            "",
        )
        .await;
        assert_eq!(html.body, bytes);
    }
    let audit = mcp_call(&t, &token, "get_audit_trail", json!({"document_id":id})).await;
    let html = request(
        &t.router,
        "GET",
        &format!("/documents/{id}/audit.json"),
        Some(&format!("session={}", t.session)),
        vec![],
        "",
    )
    .await;
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&html.body).unwrap(),
        audit["structuredContent"]
    );
    assert_eq!(
        audit["structuredContent"]["signatures"]
            .as_array()
            .unwrap()
            .len(),
        2
    );
    assert!(
        audit["structuredContent"]["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["actor"].as_str().unwrap().starts_with("mcp:"))
    );
    let d = mcp_call(&t, &token, "get_document", json!({"document_id":id})).await;
    let old_url = d["structuredContent"]["download_links"]["document_url"]
        .as_str()
        .unwrap()
        .to_string();
    let renewed = mcp_call(
        &t,
        &token,
        "renew_download_links",
        json!({"document_id":id}),
    )
    .await;
    assert_ne!(
        renewed["structuredContent"]["download_links"]["document_url"],
        old_url
    );
    let old_path = old_url.strip_prefix(&t.app.config.base_url).unwrap();
    assert_eq!(
        request(&t.router, "GET", old_path, None, vec![], "")
            .await
            .status,
        StatusCode::FORBIDDEN
    );
    sqlx::query("UPDATE downloads SET expires_at=? WHERE document_id=?")
        .bind(db::now() - 1)
        .bind(id)
        .execute(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(
        mcp_call(
            &t,
            &token,
            "get_document_file",
            json!({"document_id":id,"kind":"final"})
        )
        .await["isError"],
        false
    );
    assert_eq!(
        mcp_call(&t, &token, "cancel_document", json!({"document_id":id})).await["isError"],
        true
    );
    let draft = t.upload().await;
    assert_eq!(
        mcp_call(&t, &token, "cancel_document", json!({"document_id":draft})).await["structuredContent"]
            ["status"],
        "cancelled"
    );
    let activity = mcp_call(&t, &token, "list_admin_tokens", json!({})).await;
    let serialized = activity.to_string();
    assert!(!serialized.contains(&token));
    assert!(!serialized.contains(&old[0]));
    assert!(!serialized.contains(&pending[0]));
}
#[tokio::test]
async fn mcp_full_admin_tokens_can_create_another_token_and_revoke_themselves() {
    use serde_json::json;
    let t = setup().await;
    let first = mcp_token(&t).await;
    let created = mcp_call(
        &t,
        &first,
        "create_admin_token",
        json!({"name":"Anden AI","days":7}),
    )
    .await;
    let second = created["structuredContent"]["token"].as_str().unwrap();
    assert_ne!(first, second);
    let id: i64 = sqlx::query_scalar("SELECT id FROM api_tokens WHERE token_hash=?")
        .bind(db::hash(&first))
        .fetch_one(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(
        mcp_call(&t, &first, "revoke_admin_token", json!({"token_id":id})).await["isError"],
        false
    );
    let ping = json!({"jsonrpc":"2.0","id":1,"method":"ping"});
    assert_eq!(
        mcp_request(&t, Some(&first), "POST", ping.clone(), &[])
            .await
            .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        mcp_request(&t, Some(second), "POST", ping, &[]).await.0,
        StatusCode::OK
    );
    let raw_count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM api_tokens WHERE token_hash LIKE 'su_mcp_%'")
            .fetch_one(&t.app.pool)
            .await
            .unwrap();
    assert_eq!(raw_count, 0);
    let activity = mcp_call(&t, second, "list_admin_tokens", json!({})).await;
    assert!(!activity.to_string().contains(second));
    assert!(!activity.to_string().contains(&first));
}

#[tokio::test]
async fn mcp_token_issuance_rechecks_issuer_inside_its_transaction() {
    use document_signing::mcp::{self, Issuer};
    let t = setup().await;
    let token = mcp_token(&t).await;
    let id: i64 = sqlx::query_scalar("SELECT id FROM api_tokens WHERE token_hash=?")
        .bind(db::hash(token))
        .fetch_one(&t.app.pool)
        .await
        .unwrap();
    mcp::revoke_token(&t.app, 1, id, None)
        .await
        .unwrap_or_else(|_| panic!("Revocation failed"));
    assert!(
        mcp::create_token(&t.app, 1, "For sent", 90, Issuer::Token(id))
            .await
            .is_err()
    );
    sqlx::query("DELETE FROM sessions")
        .execute(&t.app.pool)
        .await
        .unwrap();
    assert!(
        mcp::create_token(&t.app, 1, "For sent", 90, Issuer::Session(&t.session))
            .await
            .is_err()
    );
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM api_tokens")
        .fetch_one(&t.app.pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}

#[test]
fn adversarial_pdf_indirect_page_tree_entries_can_be_completed() {
    let mut doc = Document::load_mem(&sample_pdf()).unwrap();
    let root = doc
        .catalog()
        .unwrap()
        .get(b"Pages")
        .unwrap()
        .as_reference()
        .unwrap();
    let kids = doc
        .get_dictionary(root)
        .unwrap()
        .get(b"Kids")
        .unwrap()
        .clone();
    let kids_ref = doc.add_object(kids);
    let count_ref = doc.add_object(1i64);
    let tree = doc.get_object_mut(root).unwrap().as_dict_mut().unwrap();
    tree.set("Kids", kids_ref);
    tree.set("Count", count_ref);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    assert!(pdf::validate(&bytes).is_ok());
    let signer = receipt_signer(1);
    let result = pdf::complete(
        &bytes,
        signer.document_id,
        "Indirekte sidetræ",
        &db::hash(&bytes),
        &[signer],
    );
    assert!(
        result.is_ok(),
        "PDF accepted at upload must complete: {}",
        result.err().unwrap_or_default()
    );
}

#[test]
fn adversarial_pdf_file_attachment_without_embedded_files_name_tree_is_rejected() {
    let mut doc = Document::load_mem(&sample_pdf()).unwrap();
    let payload = doc.add_object(lopdf::Stream::new(
        dictionary! {"Type"=>"EmbeddedFile"},
        b"Harmless adversarial fixture".to_vec(),
    ));
    let file = doc.add_object(dictionary!{"Type"=>"Filespec","F"=>lopdf::Object::string_literal("fixture.txt"),"EF"=>dictionary!{"F"=>payload}});
    let annotation = doc.add_object(dictionary!{"Type"=>"Annot","Subtype"=>"FileAttachment","Rect"=>vec![0.into(),0.into(),20.into(),20.into()],"FS"=>file});
    let page = *doc.get_pages().get(&1).unwrap();
    doc.get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Annots", vec![annotation.into()]);
    let mut bytes = Vec::new();
    doc.save_to(&mut bytes).unwrap();
    assert!(
        pdf::validate(&bytes).is_err(),
        "Embedded attachments must not bypass the upload policy via /EF and FileAttachment"
    );
}

#[tokio::test]
async fn adversarial_repeated_invitation_and_pdf_visits_do_not_flood_audit_history() {
    let t = setup().await;
    let id = t.upload().await;
    t.post(&format!("/documents/{id}/start"), &[]).await;
    let token = &t.tokens(id).await[0];
    for _ in 0..30 {
        assert_eq!(
            request(&t.router, "GET", &format!("/s/{token}"), None, vec![], "")
                .await
                .status,
            StatusCode::SEE_OTHER
        );
        assert_eq!(
            request(
                &t.router,
                "GET",
                "/sign/document.pdf",
                Some(&format!("signer={token}")),
                vec![],
                ""
            )
            .await
            .status,
            StatusCode::OK
        );
    }
    let count:i64=sqlx::query_scalar("SELECT count(*) FROM events WHERE document_id=? AND kind IN ('link_opened','document_accessed')").bind(id).fetch_one(&t.app.pool).await.unwrap();
    assert!(
        count <= 2,
        "Repeated visits in one short burst created {count} audit records"
    );
    sqlx::query("UPDATE events SET at=at-61 WHERE document_id=? AND kind IN ('link_opened','document_accessed')")
        .bind(id).execute(&t.app.pool).await.unwrap();
    request(&t.router, "GET", &format!("/s/{token}"), None, vec![], "").await;
    request(
        &t.router,
        "GET",
        "/sign/document.pdf",
        Some(&format!("signer={token}")),
        vec![],
        "",
    )
    .await;
    let later:i64=sqlx::query_scalar("SELECT count(*) FROM events WHERE document_id=? AND kind IN ('link_opened','document_accessed')")
        .bind(id).fetch_one(&t.app.pool).await.unwrap();
    assert_eq!(later, 4, "Later visits should still append audit evidence");
}

#[tokio::test]
async fn adversarial_mcp_rejects_invalid_access_before_reading_body() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let t = setup().await;
    for headers in [
        vec![("host", "127.0.0.1:3080")],
        vec![("host", "evil.example")],
    ] {
        let polled = Arc::new(AtomicBool::new(false));
        let observed = polled.clone();
        let body = http_body_util::Full::new(axum::body::Bytes::from_static(
            b"malformed unauthorized input",
        ))
        .map_frame(move |frame| {
            observed.store(true, Ordering::SeqCst);
            frame
        });
        let mut req = Request::builder()
            .method("POST")
            .uri("/mcp")
            .header("content-type", "application/json");
        for (name, value) in headers {
            req = req.header(name, value);
        }
        let response = t
            .router
            .clone()
            .oneshot(req.body(Body::new(body)).unwrap())
            .await
            .unwrap();
        assert!([StatusCode::UNAUTHORIZED, StatusCode::FORBIDDEN].contains(&response.status()));
        assert!(
            !polled.load(Ordering::SeqCst),
            "An unauthenticated MCP request read its body before rejection"
        );
    }
}

#[tokio::test]
async fn adversarial_audit_export_is_consistent_during_final_signature() {
    let t = setup().await;
    let id = t.upload().await;
    t.post(&format!("/documents/{id}/start"), &[]).await;
    let tokens = t.tokens(id).await;
    assert_eq!(t.sign(&tokens[0]).await.status, StatusCode::SEE_OTHER);
    let (_, snapshots) = tokio::join!(t.sign(&tokens[1]), async {
        let mut snapshots = Vec::new();
        for _ in 0..12 {
            snapshots.push(
                document_signing::admin::audit(&t.app, 1, id)
                    .await
                    .unwrap_or_else(|_| panic!("Audit failed")),
            );
        }
        snapshots
    });
    for s in snapshots {
        let completed = s["status"] == "completed";
        let all_signed = s["signatures"].as_array().unwrap().len() == 2;
        let completed_event = s["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["kind"] == "completed");
        assert_eq!(
            completed, all_signed,
            "Export mixes document status and signatures from different committed states"
        );
        assert_eq!(
            completed, completed_event,
            "Export mixes document status and completion event"
        );
        assert_eq!(
            completed,
            !s["final_sha256"].is_null(),
            "Export mixes document status and PDF hash"
        );
    }
}

#[test]
fn adversarial_pdf_rejects_inconsistent_page_count_without_panicking() {
    for count in [-1, 0, 2, i64::MAX] {
        let mut doc = Document::load_mem(&sample_pdf()).unwrap();
        let root = doc
            .catalog()
            .unwrap()
            .get(b"Pages")
            .unwrap()
            .as_reference()
            .unwrap();
        doc.get_object_mut(root)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Count", count);
        let mut bytes = Vec::new();
        doc.save_to(&mut bytes).unwrap();
        assert!(
            pdf::validate(&bytes).is_err(),
            "Inconsistent root count accepted: {count}"
        );
    }
}

#[tokio::test]
async fn adversarial_form_limits_preserve_large_pdf_uploads_in_html_and_mcp() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    let t = setup().await;
    for path in [
        "/login",
        "/sign",
        "/integrations/tokens",
        "/documents/1/start",
    ] {
        let r = request(
            &t.router,
            "POST",
            path,
            None,
            vec![b'a'; 64 * 1024 + 1],
            "application/x-www-form-urlencoded",
        )
        .await;
        assert_eq!(r.status, StatusCode::PAYLOAD_TOO_LARGE, "{path}");
    }
    let mut doc = Document::load_mem(&sample_pdf()).unwrap();
    doc.add_object(lopdf::Stream::new(dictionary! {}, vec![b'x'; 96 * 1024]));
    let mut pdf = Vec::new();
    doc.save_to(&mut pdf).unwrap();
    assert!(pdf.len() > 64 * 1024);
    let boundary = "adversarial-large-upload";
    let mut body = Vec::new();
    for (name, value) in [
        ("csrf", db::csrf(&t.session)),
        ("title", "Større PDF".into()),
        ("message", "Test".into()),
        ("recipients", "Testperson".into()),
        ("days", "7".into()),
    ] {
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n"
            )
            .as_bytes(),
        );
    }
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"pdf\"; filename=\"test.pdf\"\r\nContent-Type: application/pdf\r\n\r\n").as_bytes());
    body.extend_from_slice(&pdf);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    assert_eq!(
        request(
            &t.router,
            "POST",
            "/documents",
            Some(&format!("session={}", t.session)),
            body,
            &format!("multipart/form-data; boundary={boundary}")
        )
        .await
        .status,
        StatusCode::SEE_OTHER
    );
    let token = mcp_token(&t).await;
    let r=mcp_call(&t,&token,"create_document",serde_json::json!({"title":"Større MCP PDF","recipients":["Testperson"],"days":7,"pdf_base64":STANDARD.encode(pdf)})).await;
    assert_eq!(r["isError"], false);
}

#[tokio::test]
async fn adversarial_recipient_and_admin_token_roles_are_not_interchangeable() {
    let t = setup().await;
    let id = t.upload().await;
    t.post(&format!("/documents/{id}/start"), &[]).await;
    let token = t.tokens(id).await.remove(0);
    let admin_token = mcp_token(&t).await;
    let credentials = [
        None,
        Some(format!("signer={token}")),
        Some(format!("session={token}")),
        Some(format!("session={admin_token}")),
    ];
    for cookie in &credentials {
        for path in [
            "/new".to_string(),
            "/integrations".to_string(),
            format!("/documents/{id}"),
            format!("/documents/{id}/status"),
            format!("/documents/{id}/audit.json"),
            format!("/documents/{id}/files/original"),
        ] {
            assert_eq!(
                request(&t.router, "GET", &path, cookie.as_deref(), vec![], "")
                    .await
                    .status,
                StatusCode::FORBIDDEN,
                "{path}"
            );
        }
        for path in ["/", "/archive"] {
            assert_eq!(
                request(&t.router, "GET", path, cookie.as_deref(), vec![], "")
                    .await
                    .status,
                StatusCode::SEE_OTHER
            );
        }
    }
    let promoted = format!("su_mcp_{token}");
    assert_eq!(
        mcp_request(
            &t,
            Some(&promoted),
            "POST",
            serde_json::json!({"jsonrpc":"2.0","id":1,"method":"ping"}),
            &[]
        )
        .await
        .0,
        StatusCode::UNAUTHORIZED
    );
    assert_eq!(
        request(
            &t.router,
            "GET",
            "/sign",
            Some(&format!("signer={admin_token}")),
            vec![],
            ""
        )
        .await
        .status,
        StatusCode::FORBIDDEN
    );
}
