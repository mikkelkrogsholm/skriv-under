# Udvikling

Applikationen er ét Cargo-workspace med Axum, Tokio og SQLite. Produktnavnet er
**Skriv Under**; Rust-craten hedder fortsat `document-signing`.

## Projektets dele

| Sti | Ansvar |
| --- | --- |
| `src/main.rs` | Lokal kontoopsætning og serveropstart |
| `src/web.rs` | HTML, login, modtagersignering og adminflade |
| `src/admin.rs` | Fælles adminhandlinger for HTML og MCP |
| `src/mcp.rs` | MCP-protokol og administration af AI-tokens |
| `src/db.rs`, `src/schema.sql` | SQLite, migration og hændelser |
| `src/links.rs` | Signerings- og læselinks med HMAC |
| `src/pdf.rs` | PDF-validering og signeringsbilag |
| `tests/workflow.rs` | Workflow-, integrations- og adversarial-tests |
| `examples/receipt_preview.rs` | Generering af fiktive eksempel-PDF'er |
| `openwiki/` | Dokumentation med kildehenvisninger |

## Rust og buildoutput

`rust-toolchain.toml` låser toolchain til Rust 1.99.1 med minimal profil.
Udviklings- og testprofiler gemmer linjeinformation til backtraces og har
incremental compilation slået fra for at begrænse diskforbruget.

Kør Cargo fra projektroden, så `.cargo/config.toml` indlæses. Standardstien er
`../.rust-builds/document-signing`, fælles uden for checkoutet. Den eksisterende
installation bruger et lokalt symlink herfra til den allerede oprettede cache;
symlinket er ikke en del af repositoryet.

Vælg eventuelt din egen placering:

```sh
export CARGO_TARGET_DIR="$HOME/.cache/rust-builds/document-signing"
cargo build --locked
```

Runtime-data, PDF'er, database, nøgle og installerede produktionsbinærer skal
ligge uden for buildoutput. Flere samtidige checkouts med samme target-dir kan
vente på Cargos buildlås; vælg separate target-dirs ved behov.

## Kontrol

```sh
rustup component add rustfmt clippy
cargo fmt --all --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
./scripts/check-build-cache.sh
```

CI kører de tre Rust-kontroller med den låste toolchain og buildoutput uden for
checkoutet. CI bruger ingen AI-API'er eller produktionscredentials.

Testene bruger midlertidige databaser og fiktive PDF'er. Kør ikke forsøg mod en
rigtig datamappe. Lokale afprøvninger kan bruge en separat `DATA_DIR` og port.

## Fiktivt PDF-eksempel

```sh
cargo run --locked --example receipt_preview
```

Eksemplet gemmer `signeret-eksempel.pdf` og `signeringsbilag.pdf` i
`var/receipt-preview`. Filerne er tydeligt fiktive og bliver ikke committed.

## Diskbudget og oprydning

`scripts/check-build-cache.sh` kontrollerer et budget på 10 GiB. Scriptet bruger
`CARGO_TARGET_DIR`, hvis det er sat; ellers måler det standardmappen. Det følger
et eventuelt lokalt symlink og håndterer også en endnu ikke oprettet cache.
Budgettet er en overvågningsgrænse og håndhæver ikke en disk-kvote.

Når ingen builds kører, kan projektets buildoutput fjernes med:

```sh
cargo clean
```

Cargo gendanner buildfilerne ved næste kørsel. Slet ikke hele Cargo home, som
også kan indeholde installerede værktøjer og credentials.

## Valgfri systemd-timer

`deploy/systemd/` indeholder en bruger-timer til ugentlig kontrol mandag kl. 09.
Den logger forbruget og fejler ved overskridelse; den sletter ingen filer og
sender ingen beskeder.

Tilpas servicefilens `WorkingDirectory` til dit checkout. Installér derefter
service- og timerfilen i `~/.config/systemd/user/`, og kør:

```sh
systemctl --user daemon-reload
systemctl --user enable --now document-signing-build-cache.timer
systemctl --user list-timers document-signing-build-cache.timer
journalctl --user -u document-signing-build-cache.service -n 20
```

Hvis du bruger en anden target-dir end standarden, angives samme
`CARGO_TARGET_DIR` i servicefilens miljø.

## Dokumentation

[Wikiens indgang](../openwiki/quickstart.md) forklarer produktet og linker til
MCP og drift. Følg OpenWiki-arbejdsgangen i `AGENTS.md` ved ændringer i de
genererede sider. Wikiopdatering startes eksplicit; der er ingen planlagte
AI-kørsler i GitHub Actions.
