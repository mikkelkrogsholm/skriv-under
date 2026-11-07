use crate::config::Config;
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{
    SqlitePool,
    sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions},
};
use std::time::Duration;

pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
pub fn token() -> String {
    let mut b = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut b);
    hex::encode(b)
}
pub fn hash(bytes: impl AsRef<[u8]>) -> String {
    hex::encode(Sha256::digest(bytes.as_ref()))
}
pub fn csrf(secret: &str) -> String {
    hash(format!("document-signing/csrf/v1:{secret}"))
}
pub fn date(ts: i64) -> String {
    chrono::DateTime::from_timestamp(ts, 0)
        .map(|d| d.format("%Y-%m-%d %H:%M UTC").to_string())
        .unwrap_or_default()
}

pub fn prepare_data_dir(config: &Config) -> std::io::Result<()> {
    std::fs::create_dir_all(&config.data_dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&config.data_dir, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

pub async fn open(config: &Config) -> Result<SqlitePool, Box<dyn std::error::Error>> {
    prepare_data_dir(config)?;
    let options = SqliteConnectOptions::new()
        .filename(config.data_dir.join("documents.sqlite3"))
        .create_if_missing(true)
        .foreign_keys(true)
        .journal_mode(SqliteJournalMode::Wal)
        .busy_timeout(Duration::from_secs(10));
    // One connection serializes writes and transitions, including concurrent final signatures.
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(options)
        .await?;
    // Upgrade the old recipient table without rewriting documents, signatures or events.
    let columns = sqlx::query("PRAGMA table_info(recipients)")
        .fetch_all(&pool)
        .await?;
    use sqlx::Row;
    if columns
        .iter()
        .any(|c| c.get::<String, _>("name") == "email" && c.get::<i64, _>("notnull") == 1)
    {
        let mut connection = pool.acquire().await?;
        sqlx::query("PRAGMA foreign_keys=OFF")
            .execute(&mut *connection)
            .await?;
        let mut tx = sqlx::Acquire::begin(&mut *connection).await?;
        sqlx::raw_sql("CREATE TABLE recipients_without_required_email (
            id INTEGER PRIMARY KEY, document_id INTEGER NOT NULL REFERENCES documents(id),
            name TEXT NOT NULL, email TEXT, token_hash TEXT UNIQUE,
            status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','signed','declined')),
            signed_name TEXT, signed_at INTEGER, reason TEXT, consent TEXT, UNIQUE(document_id,email));
            INSERT INTO recipients_without_required_email SELECT id,document_id,name,email,token_hash,status,signed_name,signed_at,reason,consent FROM recipients;
            DROP TABLE recipients;
            ALTER TABLE recipients_without_required_email RENAME TO recipients;")
            .execute(&mut *tx).await?;
        tx.commit().await?;
        sqlx::query("PRAGMA foreign_keys=ON")
            .execute(&mut *connection)
            .await?;
    }
    // Existing mail queue is retained as legacy data, but can never be processed again.
    let old_queue: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name='mail_jobs')",
    )
    .fetch_one(&pool)
    .await?;
    if old_queue {
        sqlx::query(
            "UPDATE mail_jobs SET state='cancelled',body='' WHERE state IN ('pending','retry')",
        )
        .execute(&pool)
        .await?;
    }
    sqlx::raw_sql(include_str!("schema.sql"))
        .execute(&pool)
        .await?;
    Ok(pool)
}

#[derive(Clone, sqlx::FromRow)]
pub struct Document {
    pub id: i64,
    pub title: String,
    pub message: String,
    pub status: String,
    pub original_hash: String,
    pub created_at: i64,
    pub expires_at: i64,
    pub owner_id: i64,
}
#[derive(Clone, sqlx::FromRow)]
pub struct Recipient {
    pub id: i64,
    pub document_id: i64,
    pub name: String,
    pub email: Option<String>,
    pub status: String,
    pub signed_name: Option<String>,
    pub signed_at: Option<i64>,
    pub reason: Option<String>,
    pub consent: Option<String>,
}
pub const CONSENT: &str = "Jeg har læst dokumentet og accepterer at underskrive det elektronisk. Min bekræftelse knyttes til denne dokumentversion og registreres i signeringskvitteringen.";

pub async fn expire(pool: &SqlitePool) -> Result<(), sqlx::Error> {
    let at = now();
    let mut tx = pool.begin().await?;
    sqlx::query("INSERT INTO events(document_id,kind,actor,at,detail) SELECT id,'expired','system',?,'Signeringsfristen er udløbet' FROM documents WHERE status='pending' AND expires_at<=?").bind(at).bind(at).execute(&mut *tx).await?;
    sqlx::query("UPDATE documents SET status='expired' WHERE status='pending' AND expires_at<=?")
        .bind(at)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM sessions WHERE expires_at<=?")
        .bind(at)
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM login_attempts WHERE at<?")
        .bind(at - 86400)
        .execute(&mut *tx)
        .await?;
    tx.commit().await
}

// Lightweight list data: PDF blobs never need to be loaded to browse the archive.
#[derive(serde::Serialize, sqlx::FromRow)]
pub struct DocumentOverview {
    pub id: i64,
    pub title: String,
    pub status: String,
    pub created_at: i64,
    pub completed_at: Option<i64>,
    pub total: i64,
    pub signed: i64,
    pub awaiting: Option<String>,
    pub recipients: Option<String>,
}

pub async fn overview(pool: &SqlitePool, owner: i64) -> Result<Vec<DocumentOverview>, sqlx::Error> {
    sqlx::query_as("SELECT d.id,d.title,d.status,d.created_at,
        (SELECT max(at) FROM events WHERE document_id=d.id AND kind='completed') AS completed_at,
        (SELECT count(*) FROM recipients WHERE document_id=d.id) AS total,
        (SELECT count(*) FROM recipients WHERE document_id=d.id AND status='signed') AS signed,
        (SELECT group_concat(name, ', ') FROM recipients WHERE document_id=d.id AND status='pending') AS awaiting,
        (SELECT group_concat(name, ' ') FROM recipients WHERE document_id=d.id) AS recipients
        FROM documents d WHERE owner_id=? ORDER BY d.id DESC")
        .bind(owner).fetch_all(pool).await
}

impl Recipient {
    pub fn actor(&self) -> String {
        format!("recipient:{}", self.id)
    }
}

pub async fn event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    doc: i64,
    kind: &str,
    actor: &str,
    detail: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO events(document_id,kind,actor,at,detail) VALUES(?,?,?,?,?)")
        .bind(doc)
        .bind(kind)
        .bind(actor)
        .bind(now())
        .bind(detail)
        .execute(&mut **tx)
        .await?;
    Ok(())
}

/// Collapse repeated automatic visits into at most one immutable event per minute.
pub async fn access_event(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    doc: i64,
    kind: &str,
    actor: &str,
    detail: &str,
) -> Result<(), sqlx::Error> {
    let at = now();
    sqlx::query("INSERT INTO events(document_id,kind,actor,at,detail) SELECT ?,?,?,?,? WHERE NOT EXISTS(SELECT 1 FROM events WHERE document_id=? AND kind=? AND actor=? AND at>?)")
        .bind(doc).bind(kind).bind(actor).bind(at).bind(detail)
        .bind(doc).bind(kind).bind(actor).bind(at-60).execute(&mut **tx).await?;
    Ok(())
}
