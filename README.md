# Skriv Under

Et selvhostet dokumentsigneringssystem i Rust med dansk brugerflade.
Du uploader én eller flere PDF’er som en dokumentpakke, opretter personlige links og deler dem selv. Modtagerne
underskriver med navn og aktivt samtykke uden at oprette en konto.

Skriv Under har **én administrator**. Din valgte AI-klient kan få samme
administratoradgang gennem MCP med tokens, du selv opretter og tilbagekalder.

Produktion: [skrivunder.mikkelkrogsholm.dk](https://skrivunder.mikkelkrogsholm.dk).
Se [produktionsvejledningen](docs/production.md) for installation, backup og gendannelse.

## Funktioner

- Dokumentpakker med 1–10 PDF’er, én samlet underskrift og ét link pr. person.
- 1–20 underskrivere og frist på 7, 14 eller 30 dage.
- Personlige signeringslinks uden e-mailafsendelse eller modtagerkonti.
- Indbygget PDF-læser med dokumentvalg, sideskift, zoom og sidens tekst.
- Forhåndsvis valgte PDF’er direkte på uploadsiden, før kladden gemmes.
- Lyst og mørkt tema: følger systemet som standard, med manuelt valg gemt i browseren.
- Procesoverblik med underskrifter, afventende personer og registrerede hændelser.
- Afvisning, annullering og udskiftning af afventendes links.
- Signeret arkiv med søgning og adgang til færdige aftaler efter afslutning.
- PDF-bilag med navne, gemte samtykker, UTC-tidspunkter, referencer og hash for hver original.
- Revisionsspor som JSON og særskilt signeringsbilag som PDF.
- 12 MCP-værktøjer med 1:1 paritet med adminfladens funktioner.

## Kom i gang

Installér [Rust med rustup](https://rustup.rs/). Projektet bruger versionen i
[rust-toolchain.toml](rust-toolchain.toml), som rustup henter ved første Cargo-kørsel.

```sh
git clone https://github.com/mikkelkrogsholm/skriv-under.git
cd skriv-under
cargo run --locked -- init-admin
cargo run --locked
```

Åbn **http://127.0.0.1:3080**, og log ind med den konto, du netop oprettede.
`init-admin` bruges kun på en tom installation. Ændr en eksisterende konto med
`cargo run --locked -- configure-admin`; dette tilbagekalder også login-sessioner
og alle MCP-tokens. Der findes ingen indbygget standardadgangskode.

Kør Cargo fra projektroden. Buildoutput samles uden for checkoutet i
`../.rust-builds/document-signing`. Det kan ændres med `CARGO_TARGET_DIR`.
Læs [udviklingsvejledningen](docs/development.md) om cache, tests og diskbudget.

## Sådan fungerer signering

1. Log ind og vælg **Nyt dokument**.
2. Vælg 1–10 PDF’er og angiv pakkens titel, eventuel besked og underskrivernes navne.
3. Gennemse kladden og vælg **Opret signeringslinks**. Fristen starter nu.
4. Kopiér hvert personligt link og del det med den rette person.
5. Følg processen. Når alle har underskrevet, ligger aftalen i **Signeret arkiv**.

Modtageren åbner sit link, gennemser dokumentet, indtaster sit navn, accepterer
samtykketeksten og vælger **Underskriv alle N dokumenter** (eller **Underskriv dokumentet** ved én PDF). Linkåbning alene underskriver
ikke. Modtageren kan også afvise med en begrundelse.

Alle dokumenter og modtagerlisten låses ved upload. Ændringer kræver en ny runde.
Pakken må indeholde 1–10 PDF-filer, tilsammen højst 20 MiB og 200 sider. Den samlede
PDF skal også være højst 20 MiB. Originalerne bevares uændret, og deres rækkefølge,
navne, sidetal og SHA-256 følger pakken og revisionssporet. Den færdige aftale
hentes som én samlet PDF med signeringsbilag.
Adgangskodebeskyttelse, vedhæftninger,
aktivt indhold og eksisterende certifikatunderskrifter understøttes ikke.

## AI-adgang via MCP

Åbn **AI / MCP** i adminmenuen, og opret et navngivet token med udløb.
Det fulde token vises kun ved oprettelsen.

| Indstilling | Værdi |
| --- | --- |
| Transport | Streamable HTTP |
| Serveradresse i produktion | `https://skrivunder.mikkelkrogsholm.dk/mcp` |
| Serveradresse lokalt | `http://127.0.0.1:3080/mcp` |
| Header | `Authorization: Bearer DIT_TOKEN` |

Klienten skal understøtte en brugerdefineret Authorization-header. OAuth-login
er ikke implementeret. Tokenet har fuld administratoradgang, inklusive mulighed
for at oprette andre tokens. Tokens tilbagekaldes individuelt.

AI'en kan oprette dokumenter, administrere links, følge processen og hente
arkiverede PDF'er og revisionsspor. Modtagernes underskrift kræver stadig aktivt
samtykke. Se [MCP-vejledningen](openwiki/integrations/mcp.md) for alle 12 værktøjer,
PDF-upload/download og tokenlivscyklus.

MCP `create_document` accepterer enten `pdf_base64` for én PDF eller
`files: [{"name":"Aftale.pdf","pdf_base64":"…"}, …]` for en pakke.
`get_document` og `get_audit_trail` returnerer `files` med navne, sider og hashes.
`get_document_file` henter pakken med `original`/`final`, bilaget med `receipt`
eller en uændret original med `source-1` til `source-10`.

## Konfiguration og data

| Miljøvariabel | Standard | Betydning |
| --- | --- | --- |
| `BASE_URL` | `http://127.0.0.1:3080` | Oprindelsen til de links, du deler |
| `BIND` | `127.0.0.1:3080` | Serverens lytteadresse |
| `DATA_DIR` | `var` | Privat mappe til SQLite, dokumenter og linknøgle |
| `TRUST_PROXY` | `false` | Stol på én `X-Skriv-Under-Client-IP` fra en lokal proxy; kræver loopback i `BIND` |

Se [.env.example](.env.example). Appen indlæser ikke `.env` automatisk;
eksportér miljøvariablerne før opstart. `DATA_DIR` regnes fra arbejdsbiblioteket,
medmindre du bruger en absolut sti.

Ved offentlig drift skal `BASE_URL` bruge HTTPS med en TLS-proxy foran appen.
Proxyen skal bevare `Host`, så det matcher `BASE_URL`. En eventuel `Origin` skal
matche samme oprindelse. Undlad at logge Authorization, cookies, formularindhold
og stier med signerings- eller downloadtokens.

**Tag en sammenhængende backup af SQLite-databasen og `DATA_DIR/link-key`.**
Brug SQLite-backup under drift, da nyere data kan ligge i WAL. En manglende eller
forkert linknøgle kan gøre eksisterende links ubrugelige; appen kontrollerer
sammenhængen ved opstart. Runtime-data og credentials er udelukket fra Git.

Med `TRUST_PROXY=true` skal proxyen overskrive `X-Skriv-Under-Client-IP` med
forbindelsens faktiske klient-IP. Loginbegrænsningen bruger denne adresse;
manglende eller ugyldige værdier afvises. `X-Forwarded-For` bruges ikke.

Kør én serverproces pr. datamappe. Produktionsopsætningen indeholder en daglig
krypteret NAS-backup via systemd og restic. Dokumenternes slettepolitik og
webbaseret nulstilling af adgangskoder er endnu ikke implementeret.
Se [driftsvejledningen](openwiki/operations/lokal-drift.md).

## Hvad dokumenterer underskriften?

Den færdige PDF får bilaget **Underskrifter og dokumentation** med det indtastede
navn, serverregistreret UTC-tidspunkt, faktisk samtykke, dokumentreference,
signeringsreference og SHA-256 af originalen. Samme bilag kan hentes separat.

Systemet verificerer **ikke identitet med MitID** og bruger hverken en
certifikatbaseret PDF-signatur eller et uafhængigt tidsstempel. Linkåbninger
beviser ikke læsning; gentagne åbninger og PDF-visninger samles pr. person og
type i ét minut. Bilaget bruger Windows-1252; øvrige navnetegn vises som numeriske
referencer, mens fuld Unicode bevares i databasen og JSON-eksporten.

## Udvikling og dokumentation

```sh
rustup component add rustfmt clippy
cargo fmt --all --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
```

GitHub Actions kører formatering, tests og Clippy ved push og pull requests.
Testene dækker blandt andet adgangskontrol, MCP-paritet, samtykke, samtidighed,
linkrotation, databaseopgradering og adversarial-scenarier.

- [Udvikling, buildcache og diskbudget](docs/development.md)
- [Wiki: kom i gang](openwiki/quickstart.md)
- [Signeringsforløb, arkiv og PDF-bilag](openwiki/product/signering-og-arkiv.md)
- [MCP og adminparitet](openwiki/integrations/mcp.md)
- [Drift, konfiguration og backup](openwiki/operations/lokal-drift.md)
