use crate::db;
use hmac::{Hmac, Mac};
use sha2::Sha256;
use sqlx::{Sqlite, Transaction};
use std::{
    io::{Read, Write},
    path::Path,
};

pub async fn load_key(
    dir: &Path,
    pool: &sqlx::SqlitePool,
) -> Result<[u8; 32], Box<dyn std::error::Error>> {
    let path = dir.join("link-key");
    if !path.exists() {
        let existing:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM recipient_links)+(SELECT count(*) FROM shared_download_links)").fetch_one(pool).await?;
        if existing > 0 {
            return Err(
                "Linknøglen mangler. Gendan DATA_DIR/link-key fra backup, før appen startes."
                    .into(),
            );
        }
    }
    let key = load_key_file(dir)?;
    let recipients=sqlx::query_as::<_,(i64,String,String)>("SELECT r.id,l.nonce,r.token_hash FROM recipient_links l JOIN recipients r ON r.id=l.recipient_id").fetch_all(pool).await?;
    for (id, nonce, expected) in recipients {
        if db::hash(token(&key, "recipient", id, &nonce)) != expected {
            return Err("Linknøglen matcher ikke databasen. Gendan den korrekte DATA_DIR/link-key fra backup.".into());
        }
    }
    let downloads =
        sqlx::query_as::<_, (i64, String)>("SELECT document_id,nonce FROM shared_download_links")
            .fetch_all(pool)
            .await?;
    for (id, nonce) in downloads {
        let matches: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM downloads WHERE token_hash=? AND document_id=?)",
        )
        .bind(db::hash(token(&key, "download", id, &nonce)))
        .bind(id)
        .fetch_one(pool)
        .await?;
        if !matches {
            return Err("Linknøglen eller læselinkene matcher ikke databasen. Gendan et sammenhængende sæt database og link-key fra backup.".into());
        }
    }
    Ok(key)
}
fn load_key_file(dir: &Path) -> std::io::Result<[u8; 32]> {
    let path = dir.join("link-key");
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut f) => {
            let mut key = [0u8; 32];
            rand::RngCore::fill_bytes(&mut rand::rngs::OsRng, &mut key);
            f.write_all(&key)?;
            f.sync_all()?;
            Ok(key)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let mut file = std::fs::File::open(path)?;
            let mut key = [0u8; 32];
            file.read_exact(&mut key)?;
            let mut extra = [0u8; 1];
            if file.read(&mut extra)? != 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    "Invalid link key length",
                ));
            }
            Ok(key)
        }
        Err(e) => Err(e),
    }
}

// HMAC derives recoverable bearer tokens from a private key and random stored nonces.
// The database contains nonces and token hashes, never the recoverable raw token.
pub fn token(key: &[u8; 32], purpose: &str, id: i64, nonce: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(key).expect("HMAC accepts a 32-byte key");
    mac.update(format!("Skriv Under/link/v1:{purpose}:{id}:{nonce}").as_bytes());
    hex::encode(mac.finalize().into_bytes())
}

pub async fn rotate_recipient(
    tx: &mut Transaction<'_, Sqlite>,
    key: &[u8; 32],
    doc: i64,
    recipient: i64,
) -> Result<(), sqlx::Error> {
    let nonce = db::token();
    let secret = token(key, "recipient", recipient, &nonce);
    sqlx::query("INSERT INTO recipient_links(recipient_id,nonce) VALUES(?,?) ON CONFLICT(recipient_id) DO UPDATE SET nonce=excluded.nonce")
        .bind(recipient).bind(&nonce).execute(&mut **tx).await?;
    sqlx::query("UPDATE recipients SET token_hash=? WHERE id=? AND document_id=?")
        .bind(db::hash(secret))
        .bind(recipient)
        .bind(doc)
        .execute(&mut **tx)
        .await?;
    db::event(
        tx,
        doc,
        "link_created",
        &format!("recipient:{recipient}"),
        "Personligt signeringslink oprettet til manuel deling; tidligere link tilbagekaldt",
    )
    .await
}

pub async fn rotate_download(
    tx: &mut Transaction<'_, Sqlite>,
    key: &[u8; 32],
    doc: i64,
) -> Result<(), sqlx::Error> {
    let previous: Option<String> =
        sqlx::query_scalar("SELECT nonce FROM shared_download_links WHERE document_id=?")
            .bind(doc)
            .fetch_optional(&mut **tx)
            .await?;
    if let Some(previous) = previous {
        sqlx::query("DELETE FROM downloads WHERE token_hash=?")
            .bind(db::hash(token(key, "download", doc, &previous)))
            .execute(&mut **tx)
            .await?;
    }
    let nonce = db::token();
    let secret = token(key, "download", doc, &nonce);
    let expires = db::now() + 30 * 86400;
    sqlx::query("INSERT INTO shared_download_links(document_id,nonce,expires_at) VALUES(?,?,?) ON CONFLICT(document_id) DO UPDATE SET nonce=excluded.nonce,expires_at=excluded.expires_at")
        .bind(doc).bind(&nonce).bind(expires).execute(&mut **tx).await?;
    sqlx::query("INSERT INTO downloads(token_hash,document_id,expires_at) VALUES(?,?,?)")
        .bind(db::hash(secret))
        .bind(doc)
        .bind(expires)
        .execute(&mut **tx)
        .await?;
    db::event(
        tx,
        doc,
        "download_link_created",
        "afsender",
        "Læselink til færdig PDF og kvittering oprettet til manuel deling; gyldigt i 30 dage",
    )
    .await
}
