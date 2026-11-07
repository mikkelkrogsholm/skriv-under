use argon2::{Argon2, PasswordHasher, password_hash::SaltString};
use document_signing::{
    config::Config,
    db, links,
    web::{self, App},
};
use std::sync::Arc;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    if args.len() > 1 {
        return Err("Brug --help for kommandooversigt".into());
    }
    let command = args.first().map(String::as_str);
    if matches!(command, Some("--help" | "-h")) {
        println!(
            "Skriv Under\n\nBrug: document-signing [serve | init-admin | bootstrap-admin | configure-admin]\n\nUden kommando startes serveren. Kontoopsætning foretages lokalt i terminalen."
        );
        return Ok(());
    }
    if command == Some("--version") {
        println!("Skriv Under {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if !matches!(
        command,
        None | Some("serve" | "init-admin" | "bootstrap-admin" | "configure-admin")
    ) {
        return Err("Ukendt kommando. Brug --help for kommandooversigt".into());
    }
    let config = Config::load()?;
    // Take the server lock before opening or migrating its database.
    let _instance = if matches!(command, None | Some("serve")) {
        db::prepare_data_dir(&config)?;
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(config.data_dir.join("server.lock"))?;
        file.try_lock()
            .map_err(|_| "En Skriv Under-server bruger allerede denne datamappe")?;
        Some(file)
    } else {
        None
    };
    let pool = db::open(&config).await?;
    if command == Some("bootstrap-admin") {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
            .fetch_one(&pool)
            .await?;
        if count != 0 {
            return Err(
                "Der findes allerede en administrator. Brug configure-admin for at ændre din konto.".into(),
            );
        }
        let password = db::token();
        let salt = SaltString::generate(&mut rand::rngs::OsRng);
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|_| "Password hashing failed")?
            .to_string();
        let path = config.data_dir.join("initial-admin.txt");
        let mut opts = std::fs::OpenOptions::new();
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        use std::io::Write;
        let mut file = opts.open(&path)?;
        write!(
            file,
            "Local administrator\nEmail: admin@localhost\nPassword: {password}\n\nUse configure-admin to set your own login and password.\n"
        )?;
        sqlx::query("INSERT INTO users(email,password_hash) VALUES('admin@localhost',?)")
            .bind(hash)
            .execute(&pool)
            .await?;
        println!(
            "Local administrator created; credentials: {}",
            path.display()
        );
        return Ok(());
    }
    if matches!(command, Some("init-admin" | "configure-admin")) {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
            .fetch_one(&pool)
            .await?;
        if command == Some("init-admin") && count != 0 {
            return Err("Der findes allerede en administrator. Brug configure-admin for at ændre din konto.".into());
        }
        if command == Some("configure-admin") && count != 1 {
            return Err("Opret først din administrator med init-admin.".into());
        }
        use std::io::{self, Write};
        print!("Administratorens login (brugernavn eller e-mail): ");
        io::stdout().flush()?;
        let mut email = String::new();
        io::stdin().read_line(&mut email)?;
        let email = email.trim().to_lowercase();
        if email.is_empty()
            || email.len() > 254
            || email.chars().any(char::is_whitespace)
            || email.chars().any(char::is_control)
        {
            return Err("Angiv et gyldigt login-navn på højst 254 tegn uden mellemrum".into());
        }
        let password = rpassword::prompt_password("Adgangskode (mindst 12 tegn): ")?;
        if password.chars().count() < 12 || password.len() > 1024 {
            return Err("Adgangskoden skal have mindst 12 tegn og højst 1024 bytes".into());
        }
        if password != rpassword::prompt_password("Gentag adgangskode: ")? {
            return Err("Adgangskoderne er forskellige".into());
        }
        let salt = SaltString::generate(&mut rand::rngs::OsRng);
        let hash = Argon2::default()
            .hash_password(password.as_bytes(), &salt)
            .map_err(|_| "Password hashing failed")?
            .to_string();
        let mut tx = pool.begin().await?;
        if command == Some("configure-admin") {
            // Preserve the administrator ID and all document ownership.
            sqlx::query("UPDATE users SET email=?,password_hash=?")
                .bind(&email)
                .bind(hash)
                .execute(&mut *tx)
                .await?;
            sqlx::query("UPDATE api_tokens SET revoked_at=? WHERE revoked_at IS NULL")
                .bind(db::now())
                .execute(&mut *tx)
                .await?;
            sqlx::query("DELETE FROM sessions")
                .execute(&mut *tx)
                .await?;
        } else {
            sqlx::query("INSERT INTO users(email,password_hash) VALUES(?,?)")
                .bind(&email)
                .bind(hash)
                .execute(&mut *tx)
                .await?;
        }
        tx.commit().await?;
        if command == Some("configure-admin") {
            let credentials = config.data_dir.join("initial-admin.txt");
            if credentials.exists() {
                std::fs::remove_file(credentials)?;
            }
            println!(
                "Din administratorkonto er opdateret. Tidligere login-sessioner er afsluttet."
            );
        } else {
            println!("Din administratorkonto er oprettet.");
        }
        return Ok(());
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users")
        .fetch_one(&pool)
        .await?;
    if count == 0 {
        return Err("Opret først en administrator med: cargo run -- init-admin".into());
    }
    let app = Arc::new(App {
        pool: pool.clone(),
        link_key: links::load_key(&config.data_dir, &pool).await?,
        config: config.clone(),
    });
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(10));
        loop {
            tick.tick().await;
            if let Err(e) = db::expire(&pool).await {
                eprintln!("Maintenance failed: {e}");
            }
        }
    });
    let listener = tokio::net::TcpListener::bind(&app.config.bind).await?;
    println!(
        "Skriv Under: {} — del signeringslinks fra administratorportalen",
        app.config.base_url
    );
    axum::serve(
        listener,
        web::router(app).into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        let _ = tokio::signal::ctrl_c().await;
    })
    .await?;
    Ok(())
}
