CREATE TABLE IF NOT EXISTS users (
 id INTEGER PRIMARY KEY, email TEXT NOT NULL UNIQUE, password_hash TEXT NOT NULL
);
-- Every administrator row has the same index key, so at most one can exist.
CREATE UNIQUE INDEX IF NOT EXISTS single_administrator ON users ((1));
CREATE TABLE IF NOT EXISTS sessions (
 token_hash TEXT PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id), expires_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS login_attempts (id INTEGER PRIMARY KEY, ip TEXT NOT NULL, at INTEGER NOT NULL);
CREATE INDEX IF NOT EXISTS login_attempts_ip ON login_attempts(ip,at);
CREATE TABLE IF NOT EXISTS documents (
 id INTEGER PRIMARY KEY, owner_id INTEGER NOT NULL REFERENCES users(id), title TEXT NOT NULL,
 message TEXT NOT NULL, status TEXT NOT NULL DEFAULT 'draft' CHECK(status IN ('draft','pending','completed','declined','expired','cancelled')),
 pdf BLOB NOT NULL, original_hash TEXT NOT NULL, final_pdf BLOB, receipt_pdf BLOB,
 final_hash TEXT, created_at INTEGER NOT NULL, expires_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS recipients (
 id INTEGER PRIMARY KEY, document_id INTEGER NOT NULL REFERENCES documents(id), name TEXT NOT NULL, email TEXT,
 token_hash TEXT UNIQUE, status TEXT NOT NULL DEFAULT 'pending' CHECK(status IN ('pending','signed','declined')),
 signed_name TEXT, signed_at INTEGER, reason TEXT, consent TEXT, UNIQUE(document_id,email)
);
CREATE TABLE IF NOT EXISTS events (
 id INTEGER PRIMARY KEY, document_id INTEGER NOT NULL REFERENCES documents(id), kind TEXT NOT NULL,
 actor TEXT NOT NULL, at INTEGER NOT NULL, detail TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS downloads (
 token_hash TEXT PRIMARY KEY, document_id INTEGER NOT NULL REFERENCES documents(id), expires_at INTEGER NOT NULL
);
-- Nonces allow the administrator to recover links with a separate local secret key.
CREATE TABLE IF NOT EXISTS recipient_links (
 recipient_id INTEGER PRIMARY KEY REFERENCES recipients(id), nonce TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS shared_download_links (
 document_id INTEGER PRIMARY KEY REFERENCES documents(id), nonce TEXT NOT NULL, expires_at INTEGER NOT NULL
);
-- Full administrator access for MCP clients. Raw bearer tokens are never persisted.
CREATE TABLE IF NOT EXISTS api_tokens (
 id INTEGER PRIMARY KEY, user_id INTEGER NOT NULL REFERENCES users(id), name TEXT NOT NULL,
 token_hash TEXT NOT NULL UNIQUE, prefix TEXT NOT NULL, created_at INTEGER NOT NULL,
 expires_at INTEGER NOT NULL, last_used_at INTEGER, revoked_at INTEGER
);
CREATE TABLE IF NOT EXISTS admin_access_events (
 id INTEGER PRIMARY KEY, token_id INTEGER REFERENCES api_tokens(id), kind TEXT NOT NULL,
 at INTEGER NOT NULL, detail TEXT NOT NULL
);

-- Access deduplication and per-recipient progress use this bounded lookup.
CREATE INDEX IF NOT EXISTS events_access ON events (document_id, kind, actor, at);
