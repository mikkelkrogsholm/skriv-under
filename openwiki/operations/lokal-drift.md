---
type: operations
title: Drift, links og buildplads
description: Start Skriv Under, vedligehold den eneste administratorkonto, beskyt nøglen til genkopierbare links og hold SQLite-data adskilt fra Rust-buildfiler.
tags: [operations, configuration, links, storage, rust]
verified:
  - by: openwiki/0.9.0
    at: 2026-11-08T16:42:19.277Z
sources:
  - id: openwiki-source-4905fab56ecf9fa5e1ebbf3f
    resource: repo://.cargo/config.toml
  - id: openwiki-source-164e2da859b5277df81c7d94
    resource: repo://.github/workflows/ci.yml
  - id: openwiki-source-651d1fb6c9e49916a916ab51
    resource: repo://Cargo.toml
  - id: openwiki-source-024ebbcbb14be96ebf905d39
    resource: repo://deploy/caddy/SkrivUnder.Caddyfile
  - id: openwiki-source-dbee0aeefb17a915f02e6e73
    resource: repo://deploy/systemd/document-signing-build-cache.service
  - id: openwiki-source-cd30c5dcd48ca4d1c0f6b7e5
    resource: repo://deploy/systemd/document-signing-build-cache.timer
  - id: openwiki-source-7c0bfb7e4ab911badbcd3424
    resource: repo://deploy/systemd/skriv-under-backup.timer
  - id: openwiki-source-c7888eeba3ff0ecf2239cad6
    resource: repo://deploy/systemd/skriv-under.service
  - id: openwiki-source-70991026f1538e8215a8789e
    resource: repo://docs/development.md
  - id: openwiki-source-d5037beb9bfa41488c60312e
    resource: repo://docs/production.md
  - id: openwiki-source-b7793decf9d7c9ba48e57e0f
    resource: repo://rust-toolchain.toml
  - id: openwiki-source-e6df842e00830694a2fe45f0
    resource: repo://scripts/backup-production.sh
  - id: openwiki-source-7c8eb8aa4e6191d12b8c0341
    resource: repo://scripts/check-build-cache.sh
  - id: openwiki-source-2a737474d86fc75cc9d9694f
    resource: repo://src/config.rs
  - id: openwiki-source-1e35ba9c5eb8fd2aeed68a14
    resource: repo://src/db.rs
  - id: openwiki-source-2d54f64140ade8c180921259
    resource: repo://src/links.rs
  - id: openwiki-source-b55a21a31ede1b56cd31a6a6
    resource: repo://src/main.rs
  - id: openwiki-source-b95ec25c5cfc0be350df258c
    resource: repo://src/mcp.rs
  - id: openwiki-source-10dbc7a48ac0283046204658
    resource: repo://src/schema.sql
  - id: openwiki-source-7b75006986d8b0337adc6ca0
    resource: repo://src/web.rs
  - id: openwiki-source-a4d0d6fea18ea7627c71affd
    resource: repo://tests/workflow.rs
generated: { by: "codex", at: "2026-11-08T16:42:19.277Z" }
---

# Drift, links og buildplads

Skriv Under køres som én Rust-proces. Start fra projektroden, så projektets Cargo-konfiguration og de relative runtime-stier bliver brugt.

## Start og administratorkonto

```sh
cargo run --locked
```

Åbn `http://127.0.0.1:3080`, og brug den eksisterende konto. Kun på en tom installation oprettes administratoren med `cargo run --locked -- init-admin`. Alternativt opretter `bootstrap-admin` en tilfældig adgangskode i den private fil `var/initial-admin.txt`.

`cargo run --locked -- configure-admin` ændrer dit eksisterende login og din adgangskode. Kontoens ID og dokumentejerskab bevares, alle login-sessioner afsluttes, alle MCP-tokens tilbagekaldes, og bootstrap-filen fjernes. Der kan højst være én administrator. Login kan være et brugernavn eller en e-mailadresse; det bruges kun til login og aldrig til afsendelse. Adgangskoden skal være mindst 12 tegn og højst 1024 bytes. Et login, der er under adgangskodekontrol under et adgangskodeskift, kan heller ikke oprette en gyldig session bagefter.

`cargo run --locked -- --help` og `--version` starter ikke serveren og opretter ingen runtime-data. Ukendte kommandoer afvises.

## Konfiguration

Appen sender ingen e-mails og kræver ingen SMTP-konfiguration. Administratoren kopierer personlige links fra dokumentets side og deler dem selv.

| Variabel | Standard og betydning |
| --- | --- |
| `BASE_URL` | `http://127.0.0.1:3080`; oprindelsen til de links, du deler |
| `BIND` | `127.0.0.1:3080`; serverens lytteadresse |
| `DATA_DIR` | `var`; privat mappe til database og linknøgle |
| `TRUST_PROXY` | `false`; aktivér kun bag lokal proxy, der overskriver klient-IP |

`BASE_URL` normaliseres til en oprindelse; HTTPS giver Secure-cookies også ved store bogstaver i den indtastede adresse.

Miljøvariabler læses ved opstart. `.env` indlæses ikke automatisk. Hvis filen bruges, eksporteres værdierne eksplicit før opstart. Ved offentlig drift skal `BASE_URL` bruge HTTPS, og en TLS-proxy skal stå foran appen. Undlad at logge cookies, formularindhold og stier med `/s/` eller `/d/`. Loginbegrænsningen bruger forbindelsens faktiske IP. Med `TRUST_PROXY=true` kræves loopback i `BIND`, og én gyldig `X-Skriv-Under-Client-IP` fra den lokale proxy bruges i stedet. Manglende, ugyldige eller gentagne værdier afvises; `X-Forwarded-For` ignoreres.

## AI-klienter og MCP

Administratoren opretter navngivne, tidsbegrænsede tokens på **AI / MCP**. Klienten bruger `BASE_URL/mcp` med Streamable HTTP og `Authorization: Bearer DIT_TOKEN`. Der kræves ingen yderligere miljøvariabler. Tokens har fuld adminparitet, og adgangskodeskift med `configure-admin` tilbagekalder alle eksisterende tokens.

Ved reverse proxy skal `Host` bevares og matche `BASE_URL`; en eventuel `Origin` skal være samme oprindelse. Log ikke Authorization-headeren. Tokenværdien vises kun ved oprettelse. SQLite gemmer hash, kort præfiks, udløb, seneste brug og tilbagekaldelse samt et aktivitetsspor uden rå argumenter eller dokumentindhold.

Tilbagekaldelse i adminfladen afviser efterfølgende forespørgsler. Tokens administreres enkeltvis, også når en AI har oprettet flere. Se [MCP-forbindelse og adminparitet](../integrations/mcp.md) for klientkrav, alle værktøjer og kontrol af udstederen under tokenoprettelse.

## Ressourcegrænser

MCP kontrollerer Host, Origin og token før læsning af request-body. Almindelige formularer har en grænse på 64 KiB, PDF-multipart 22 MiB og MCP 30 MiB til base64; selve PDF-filen er stadig højst 20 MiB. Metadataopslag vælger kun metadatafelter, så proceskontrol ikke indlæser store PDF-BLOBs.

Gentagne linkåbninger og PDF-visninger samles til højst én revisionshændelse pr. modtager og type pr. minut. Hændelserne forbliver uændrede; senere besøg tilføjer nye hændelser. Et indeks på dokument, hændelsestype, aktør og tidspunkt understøtter kontrollen. Dette er en begrænsning af logvækst, ikke en generel beskyttelse mod trafikoverbelastning.

## Linknøgle, backup og migration

`DATA_DIR/link-key` indeholder en privat 32-byte nøgle med rettighederne `0600` ved oprettelse på Unix. Linktokens udledes med HMAC-SHA-256 fra nøglen, formålet, modtagerens eller dokumentets ID og en tilfældig nonce. Databasen gemmer noncer og tokenhashes. Administratoren kan derfor kopiere samme link igen efter genstart, uden at rå links gemmes i modtagerrækkerne.

Tag backup af både SQLite-databasen og `link-key`. Hvis nøglen mangler, mens der allerede findes genererede links, stopper appen opstarten med en besked om at gendanne nøglen. En eksisterende nøgle kontrolleres mod de gemte signerings- og læselinks, så også en forkert nøgle på præcis 32 bytes afvises. Nøglen må ikke ligge i Git, wikien, logs eller buildmappen.

Opgradering fra e-mailversionen gør modtagerens historiske e-mailfelt valgfrit. Nye invitationer bruger kun navne. Eksisterende dokumenter, PDF'er, underskrifter og hændelser bevares. Gamle mailkøer deaktiveres, og uafsendte beskeder tømmes; der findes ingen mailarbejder, der kan sende dem. Gamle udbakkefiler kan fortsat findes som private historiske filer, men læses og sendes ikke af appen.

## Data og procesdrift

SQLite med WAL gemmer dokumenter, PDF'er, underskrifter, sessioner, linkmetadata og revisionsspor i `DATA_DIR/documents.sqlite3`. En fil-lås tages før åbning eller migrering af databasen og hindrer to serverprocesser i samme datamappe. Vedligeholdelsesarbejderen kontrollerer udløb og sessionsoprydning hvert 10. sekund.

Genstart sletter ikke signerede dokumenter. Brug SQLite-backup under drift frem for kun at kopiere hovedfilen, da nyere data kan ligge i WAL. Produktionsopsætningen har en daglig krypteret NAS-backup via systemd og restic; dokumenternes slettepolitik er endnu ikke implementeret. [Integrationstestene](../../tests/workflow.rs) efterprøver både migration, manglende linknøgle og genkopiering efter genstart.

Se også [signeringsforløb og arkiv](../product/signering-og-arkiv.md) og implementeringen i [links](../../src/links.rs), [database](../../src/db.rs) og [opstart](../../src/main.rs).

## Produktion

Den offentlige installation er [skrivunder.mikkelkrogsholm.dk](https://skrivunder.mikkelkrogsholm.dk). Den kører som en separat systembruger med data i `/var/lib/skriv-under` og en versioneret binær under `/opt/skriv-under`. Caddy videresender til loopback og genindlæses ved konfigurationsændringer. Sitet undlader accesslogs og bruger generiske proxyfejl for at beskytte tokenlinks.

`skriv-under-backup.timer` tager dagligt kl. 03:35–03:40 dansk tid en SQLite online-backup sammen med linknøglen. Snapshot kontrolleres og sendes krypteret til et særskilt restic-repository på NAS. De seneste syv lokale snapshots beholdes; NAS beholder 14 daglige, otte ugentlige og 12 månedlige snapshots. NAS-mountet skal være tilgængeligt, ellers fejler backupjobbet.

Se [produktionsvejledningen](../../docs/production.md) for servicekommandoer, kontoændring, releases, backupnøgle og gendannelse.

## Buildplads og kontrol

Rust-versionen er låst til 1.99.1 med minimal installationsprofil. Cargo placerer som standard buildoutput i `../.rust-builds/document-signing`, uden for checkoutet. `CARGO_TARGET_DIR` kan vælge en anden placering. Udviklings- og testprofiler gemmer linjeinformation til backtraces og har incremental compilation slået fra. Det sparer diskplads og giver langsommere genkompilering.

Det valgfrie script og bruger-timeren `document-signing-build-cache.timer` kontrollerer et budget på 10 GiB; timeren kører mandag kl. 09. Scriptet bruger samme target-dir og følger et lokalt symlink, hvis et sådant bruges. Det er en overvågningsgrænse; kontrollen sletter ikke filer og håndhæver ingen disk-kvote.

```sh
./scripts/check-build-cache.sh
systemctl --user list-timers document-signing-build-cache.timer
cargo fmt --check
cargo test --locked
```

Når ingen builds kører, kan `cargo clean` fra projektroden fjerne projektets buildoutput. Runtime-data og kørende produktionsinstallationer skal ligge uden for buildmappen.

Servicefilens `WorkingDirectory` skal tilpasses checkoutets placering før installation af timeren. Se [udviklingsvejledningen](../../docs/development.md) for portable stier, kontrol og installation.

GitHub Actions kører formatering, tests og Clippy ved push og pull requests med read-only repositoryadgang. Workflowet bruger den låste Rust-version og ingen AI-API'er eller produktionscredentials. Wikiopdatering startes eksplicit.

Til produktets indgang: [Skriv Under – kom i gang](../quickstart.md).
