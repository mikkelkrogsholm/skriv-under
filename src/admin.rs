use crate::{
    db::{self, Document, DocumentOverview, Recipient},
    links, pdf,
    web::{App, Error, bad, missing, owned, recipients},
};
use serde_json::{Value, json};
use sqlx::Row;
type Result<T> = std::result::Result<T, Error>;

pub struct NewDocument {
    pub title: String,
    pub message: String,
    pub recipients: String,
    pub days: i64,
    pub pdf: Vec<u8>,
}
pub struct UploadFile {
    pub name: String,
    pub bytes: Vec<u8>,
}
pub async fn create_document(
    app: &App,
    user: i64,
    actor: &str,
    mut input: NewDocument,
) -> Result<i64> {
    let bytes = std::mem::take(&mut input.pdf);
    create_package(
        app,
        user,
        actor,
        input,
        vec![UploadFile {
            name: "Dokument.pdf".into(),
            bytes,
        }],
    )
    .await
}
pub async fn create_package(
    app: &App,
    user: i64,
    actor: &str,
    input: NewDocument,
    files: Vec<UploadFile>,
) -> Result<i64> {
    let title = input.title.as_str().trim();
    let message = input.message.as_str().trim();
    if title.is_empty()
        || title.chars().count() > 150
        || title.chars().any(char::is_control)
        || message.chars().count() > 4000
    {
        return Err(bad("Kontrollér titel og besked"));
    }
    let days: i64 = input.days;
    if ![7, 14, 30].contains(&days) {
        return Err(bad("Ugyldig frist"));
    }
    let mut people = Vec::new();
    let mut names = std::collections::HashSet::new();
    for line in input.recipients.lines().filter(|l| !l.trim().is_empty()) {
        let name = line.trim();
        if name.is_empty()
            || name.chars().count() > 120
            || name.chars().any(char::is_control)
            || name.contains([';', '@'])
            || !names.insert(name.to_lowercase())
        {
            return Err(bad(
                "Angiv ét unikt navn pr. linje, højst 120 tegn. E-mailadresser er ikke nødvendige.",
            ));
        }
        people.push(name.to_string());
    }
    if people.is_empty() || people.len() > 20 {
        return Err(bad("Tilføj mellem 1 og 20 underskrivere"));
    }
    if files.is_empty()
        || files.len() > 10
        || files.iter().map(|f| f.bytes.len()).sum::<usize>() > 20 * 1024 * 1024
    {
        return Err(bad(
            "Vælg 1–10 PDF-filer på tilsammen højst 20 MiB og 200 sider",
        ));
    }
    let (bytes, files, metadata) = tokio::task::spawn_blocking(move || {
        let mut metadata = Vec::new();
        let mut first_page = 1;
        for (index, file) in files.iter().enumerate() {
            let name = file.name.trim();
            if name.is_empty() || name.chars().count() > 200 || name.chars().any(char::is_control) {
                return Err("Filnavne skal være 1–200 tegn uden kontroltegn".to_string());
            }
            pdf::validate(&file.bytes)?;
            let pages = pdf::page_count(&file.bytes)? as i64;
            metadata.push(db::SourceFile {
                position: index as i64 + 1,
                name: name.into(),
                sha256: db::hash(&file.bytes),
                pages,
                first_page,
            });
            first_page += pages;
        }
        if first_page > 201 {
            return Err("Pakken må højst have 200 sider".into());
        }
        let bytes = pdf::combine(&files.iter().map(|f| f.bytes.as_slice()).collect::<Vec<_>>())?;
        pdf::validate(&bytes)?;
        Ok::<_, String>((bytes, files, metadata))
    })
    .await
    .map_err(|_| bad("PDF-valideringen fejlede"))?
    .map_err(|s| bad(&s))?;
    let digest = db::hash(&bytes);
    let mut tx = app.pool.begin().await?;
    let id=sqlx::query("INSERT INTO documents(owner_id,title,message,pdf,original_hash,created_at,expires_at) VALUES(?,?,?,?,?,?,?)").bind(user).bind(title).bind(message).bind(bytes).bind(&digest).bind(db::now()).bind(days*86400).execute(&mut *tx).await?.last_insert_rowid();
    for (file, meta) in files.into_iter().zip(&metadata) {
        sqlx::query("INSERT INTO document_files(document_id,position,name,sha256,pages,first_page,pdf) VALUES(?,?,?,?,?,?,?)")
            .bind(id).bind(meta.position).bind(&meta.name).bind(&meta.sha256).bind(meta.pages).bind(meta.first_page).bind(file.bytes).execute(&mut *tx).await?;
    }
    for name in people {
        sqlx::query("INSERT INTO recipients(document_id,name) VALUES(?,?)")
            .bind(id)
            .bind(name)
            .execute(&mut *tx)
            .await?;
    }
    db::event(
        &mut tx,
        id,
        "created",
        actor,
        &format!("Kladde oprettet. SHA-256: {digest}"),
    )
    .await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn change_document(
    app: &App,
    user: i64,
    actor: &str,
    id: i64,
    action: &str,
) -> Result<()> {
    db::expire(&app.pool).await?;
    owned(app, id, user).await?;
    let mut tx = app.pool.begin().await?;
    let mut d: Document = sqlx::query_as("SELECT id,owner_id,title,message,status,original_hash,created_at,expires_at FROM documents WHERE id=?")
        .bind(id)
        .fetch_one(&mut *tx)
        .await?;
    match action {
        "start" | "rotate" => {
            if action == "start" && d.status != "draft"
                || action == "rotate" && (d.status != "pending" || d.expires_at <= db::now())
            {
                return Err(bad("Links kan ikke oprettes i denne tilstand"));
            }
            let last: Option<i64> = sqlx::query_scalar(
                "SELECT max(at) FROM events WHERE document_id=? AND kind='link_created'",
            )
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
            if last.is_some_and(|t| db::now() - t < 60) {
                return Err(bad("Vent mindst 60 sekunder før links udskiftes"));
            }
            if action == "start" {
                d.expires_at += db::now();
                d.status = "pending".into();
                sqlx::query("UPDATE documents SET status='pending',expires_at=? WHERE id=?")
                    .bind(d.expires_at)
                    .bind(id)
                    .execute(&mut *tx)
                    .await?;
                db::event(
                    &mut tx,
                    id,
                    "started",
                    actor,
                    "Dokumentversion låst og signeringsrunde startet",
                )
                .await?;
            }
            let people: Vec<Recipient> =
                sqlx::query_as("SELECT * FROM recipients WHERE document_id=? AND status='pending'")
                    .bind(id)
                    .fetch_all(&mut *tx)
                    .await?;
            for r in people {
                links::rotate_recipient(&mut tx, &app.link_key, d.id, r.id).await?;
            }
            if action == "rotate" {
                db::event(
                    &mut tx,
                    id,
                    "links_rotated",
                    actor,
                    "Signeringslinks til afventende underskrivere udskiftet",
                )
                .await?;
            }
        }
        "cancel" => {
            if !["draft", "pending"].contains(&d.status.as_str()) {
                return Err(bad("Runden er allerede lukket"));
            }
            sqlx::query("UPDATE documents SET status='cancelled' WHERE id=?")
                .bind(id)
                .execute(&mut *tx)
                .await?;
            db::event(
                &mut tx,
                id,
                "cancelled",
                actor,
                "Signeringsrunde annulleret; links kan ikke bruges til signering",
            )
            .await?;
        }
        "renew-download" => {
            if d.status != "completed" {
                return Err(bad("Kun færdige aftaler kan få læselinks"));
            }
            links::rotate_download(&mut tx, &app.link_key, d.id).await?;
            db::event(
                &mut tx,
                id,
                "download_links_renewed",
                actor,
                "Læselinks til færdig aftale udskiftet",
            )
            .await?;
        }
        _ => return Err(missing()),
    }
    tx.commit().await?;
    Ok(())
}

pub async fn audit(app: &App, user: i64, id: i64) -> Result<Value> {
    let mut tx = app.pool.begin().await?;
    let d: Document = sqlx::query_as("SELECT id,owner_id,title,message,status,original_hash,created_at,expires_at FROM documents WHERE id=? AND owner_id=?")
        .bind(id)
        .bind(user)
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(missing)?;
    let rows =
        sqlx::query("SELECT kind,actor,at,detail FROM events WHERE document_id=? ORDER BY id")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
    let events=rows.into_iter().map(|r|serde_json::json!({"kind":r.get::<String,_>("kind"),"actor":r.get::<String,_>("actor"),"at":db::date(r.get("at")),"timestamp":r.get::<i64,_>("at"),"detail":r.get::<String,_>("detail")})).collect::<Vec<_>>();
    let final_hash: Option<String> =
        sqlx::query_scalar("SELECT final_hash FROM documents WHERE id=?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await?;
    let people: Vec<Recipient> =
        sqlx::query_as("SELECT * FROM recipients WHERE document_id=? ORDER BY id")
            .bind(id)
            .fetch_all(&mut *tx)
            .await?;
    let signatures=people.into_iter().filter(|r|r.status=="signed").map(|r|serde_json::json!({"signing_reference":format!("SU-{id}-{}",r.id),"recipient_id":r.id,"recipient_name":r.name,"signed_name":r.signed_name,"signed_at":r.signed_at,"consent":r.consent})).collect::<Vec<_>>();
    let files: Vec<db::SourceFile> = sqlx::query_as("SELECT position,name,sha256,pages,first_page FROM document_files WHERE document_id=? ORDER BY position").bind(id).fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(
        serde_json::json!({"files":files,"document_id":id,"document_reference":format!("SU-{id}"),"title":d.title,"status":d.status,"original_sha256":d.original_hash,"final_sha256":final_hash,"signatures":signatures,"events":events}),
    )
}

pub struct Listing {
    pub documents: Vec<DocumentOverview>,
    pub total: usize,
    pub page: usize,
    pub pages: usize,
    pub all_count: usize,
    pub pending: usize,
    pub completed: usize,
}
pub async fn list_documents(
    app: &App,
    user: i64,
    q: &str,
    status: &str,
    page: usize,
    archive: bool,
) -> Result<Listing> {
    if q.chars().count() > 200 {
        return Err(bad("Søgningen må højst være 200 tegn"));
    }
    if ![
        "",
        "draft",
        "pending",
        "completed",
        "declined",
        "expired",
        "cancelled",
    ]
    .contains(&status)
    {
        return Err(bad("Ukendt statusfilter"));
    }
    db::expire(&app.pool).await?;
    let mut docs = db::overview(&app.pool, user).await?;
    if archive {
        docs.sort_by_key(|d| std::cmp::Reverse((d.completed_at.unwrap_or(0), d.id)));
    }
    let all_count = docs.len();
    let pending = docs.iter().filter(|d| d.status == "pending").count();
    let completed = docs.iter().filter(|d| d.status == "completed").count();
    let q = q.trim().to_lowercase();
    docs.retain(|d| {
        (!archive || d.status == "completed")
            && (archive || status.is_empty() || status == d.status)
            && (q.is_empty()
                || d.title.to_lowercase().contains(&q)
                || d.recipients
                    .as_deref()
                    .unwrap_or("")
                    .to_lowercase()
                    .contains(&q))
    });
    let total = docs.len();
    let pages = total.div_ceil(25).max(1);
    let page = page.max(1).min(pages);
    let documents = docs.into_iter().skip((page - 1) * 25).take(25).collect();
    Ok(Listing {
        documents,
        total,
        page,
        pages,
        all_count,
        pending,
        completed,
    })
}
pub async fn file_bytes(app: &App, id: i64, kind: &str) -> Result<Vec<u8>> {
    if let Some(position) = kind
        .strip_prefix("source-")
        .and_then(|s| s.parse::<u32>().ok())
    {
        return sqlx::query_scalar(
            "SELECT pdf FROM document_files WHERE document_id=? AND position=?",
        )
        .bind(id)
        .bind(position)
        .fetch_optional(&app.pool)
        .await?
        .ok_or_else(missing);
    }
    let column = match kind {
        "original" => "pdf",
        "final" => "final_pdf",
        "receipt" => "receipt_pdf",
        _ => return Err(missing()),
    };
    let bytes: Option<Vec<u8>> =
        sqlx::query_scalar(&format!("SELECT {column} FROM documents WHERE id=?"))
            .bind(id)
            .fetch_optional(&app.pool)
            .await?
            .flatten();
    bytes.ok_or_else(missing)
}
/// Administrator detail data, including only the links currently available in the UI.
pub async fn document(app: &App, user: i64, id: i64) -> Result<Value> {
    db::expire(&app.pool).await?;
    let d = owned(app, id, user).await?;
    let people = recipients(app, id).await?;
    let mut rows = Vec::new();
    for r in people {
        let nonce: Option<String> =
            sqlx::query_scalar("SELECT nonce FROM recipient_links WHERE recipient_id=?")
                .bind(r.id)
                .fetch_optional(&app.pool)
                .await?;
        let signing_url = if d.status == "pending" && r.status == "pending" {
            nonce.map(|nonce| {
                format!(
                    "{}/s/{}",
                    app.config.base_url,
                    links::token(&app.link_key, "recipient", r.id, &nonce)
                )
            })
        } else {
            None
        };
        rows.push(json!({"id":r.id,"name":r.name,"status":r.status,"signed_name":r.signed_name,
            "signed_at":r.signed_at,"consent":r.consent,"reason":r.reason,"signing_url":signing_url}));
    }
    let mut download_links = Value::Null;
    if d.status == "completed" {
        let shared: Option<(String, i64)> = sqlx::query_as(
            "SELECT nonce,expires_at FROM shared_download_links WHERE document_id=?",
        )
        .bind(id)
        .fetch_optional(&app.pool)
        .await?;
        if let Some((nonce, expires)) = shared
            && expires > db::now()
        {
            let token = links::token(&app.link_key, "download", id, &nonce);
            download_links = json!({"document_url":format!("{}/d/{token}/final",app.config.base_url),
                "receipt_url":format!("{}/d/{token}/receipt",app.config.base_url),"expires_at":expires});
        }
    }
    let audit = audit(app, user, id).await?;
    let revision: i64 =
        sqlx::query_scalar("SELECT coalesce(max(id),0) FROM events WHERE document_id=?")
            .bind(id)
            .fetch_one(&app.pool)
            .await?;
    Ok(
        json!({"id":id,"document_reference":format!("SU-{id}"),"title":d.title,"message":d.message,
        "status":d.status,"created_at":d.created_at,"expires_at":if d.status=="draft" {Value::Null} else {json!(d.expires_at)},
        "deadline_days":if d.status=="draft" {json!(d.expires_at/86400)} else {Value::Null},
        "original_sha256":d.original_hash,"final_sha256":audit["final_sha256"],"files":audit["files"],"recipients":rows,
        "download_links":download_links,"revision":revision,"events":audit["events"]}),
    )
}
