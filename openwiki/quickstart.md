---
type: quickstart
title: Skriv Under – kom i gang
description: Skriv Under har én administrator, manuelt delte links, procesoverblik og arkiv med PDF-bilag, der dokumenterer underskrifter og godkendelser.
tags: [quickstart, product, signing, access-control]
verified:
  - by: openwiki/0.9.0
    at: 2026-11-08T16:42:19.277Z
sources:
  - id: openwiki-source-4905fab56ecf9fa5e1ebbf3f
    resource: repo://.cargo/config.toml
  - id: openwiki-source-d5037beb9bfa41488c60312e
    resource: repo://docs/production.md
  - id: openwiki-source-abe5fae4c5d81d8288df4316
    resource: repo://examples/receipt_preview.rs
  - id: openwiki-source-23775c3de52f3ab95a13cb8b
    resource: repo://README.md
  - id: openwiki-source-b7793decf9d7c9ba48e57e0f
    resource: repo://rust-toolchain.toml
  - id: openwiki-source-b6b7bc17569f2d75c652ef4d
    resource: repo://src/admin.rs
  - id: openwiki-source-2a737474d86fc75cc9d9694f
    resource: repo://src/config.rs
  - id: openwiki-source-1e35ba9c5eb8fd2aeed68a14
    resource: repo://src/db.rs
  - id: openwiki-source-b55a21a31ede1b56cd31a6a6
    resource: repo://src/main.rs
  - id: openwiki-source-b95ec25c5cfc0be350df258c
    resource: repo://src/mcp.rs
  - id: openwiki-source-efa620b89ed22813d5a01dbf
    resource: repo://src/pdf.rs
  - id: openwiki-source-10dbc7a48ac0283046204658
    resource: repo://src/schema.sql
  - id: openwiki-source-7b75006986d8b0337adc6ca0
    resource: repo://src/web.rs
generated: { by: "codex", at: "2026-11-08T16:42:19.277Z" }
---

# Skriv Under

**Skriv Under** er et dansk dokumentsigneringssystem. Ejeren uploader en PDF, opretter personlige links til manuel deling, følger underskrifterne og finder den færdige aftale i sit signerede arkiv.

Den offentlige installation ligger på [skrivunder.mikkelkrogsholm.dk](https://skrivunder.mikkelkrogsholm.dk). Se [produktionsvejledningen](../docs/production.md) for drift, backup og gendannelse.

## Hvem bruger hvad?

| Rolle | Adgang | Handlinger |
| --- | --- | --- |
| Dig som eneste administrator | Login og adgangskode | Oprette links, følge processen, annullere, erstatte links og hente arkiverede aftaler |
| Din valgte AI-klient | Navngivet MCP-token med udløb | Samme muligheder som adminfladen, inklusive tokenadministration |
| Modtager | Personligt tokenlink til ét dokument | Se dokumentet, underskrive med navn og samtykke eller afvise med begrundelse |

Modtagere får ingen konto og kan ikke tilgå dit arkiv eller dine øvrige dokumenter. Databasen tillader højst én administrator.

## Det almindelige forløb

1. Log ind, og vælg **Nyt dokument**.
2. Upload en PDF og angiv underskrivernes navne, ét pr. linje.
3. Gennemse kladden, vælg **Opret signeringslinks**, og kopiér linket til hver modtager. Del selv links gennem den kanal, du vil bruge.
4. Følg underskrifter, afventende personer og hændelser på dokumentets side. Status opdateres automatisk, mens siden er synlig.
5. Når alle har underskrevet, hentes aftalen i **Signeret arkiv** sammen med kvitteringen og proceshistorikken.

En linkåbning registreres som en åbning og kan komme fra en scanner. Gentagne åbninger og PDF-visninger samles pr. modtager og type i ét minut. Selve underskriften kræver modtagerens aktive bekræftelse. Den færdige PDF får bilaget **Underskrifter og dokumentation** med indtastede navne, signeringsreferencer, præcise tidspunkter, gemte samtykker og originalens dokumenthash. Bilaget kan fortsætte over flere sider og hentes separat; der foretages ingen MitID-kontrol eller certifikatbaseret PDF-forsegling.

## Installér og start

```sh
git clone https://github.com/mikkelkrogsholm/skriv-under.git
cd skriv-under
cargo run --locked -- init-admin
cargo run --locked
```

Åbn `http://127.0.0.1:3080`, og brug din nye administratorkonto. `init-admin` bruges kun på en tom installation; ændringer af en eksisterende konto sker med `configure-admin`. Rustup bruger toolchain-versionen fra repositoryet.

Appen sender ingen e-mails og kræver ingen modtager-e-mailadresser. Links kan kopieres igen fra administratorens dokumentside. Når aftalen er færdig, kan du også dele læselinks til PDF og kvittering. Brug [driftsvejledningen](operations/lokal-drift.md) til kontoopsætning og backup af linknøglen.

Buildoutput samles uden for checkoutet i `../.rust-builds/document-signing`, med mulighed for at vælge en anden sti via `CARGO_TARGET_DIR`. Se [udviklingsvejledningen](../docs/development.md) for tests, CI og diskbudget.

## Forbind en AI

Vælg **AI / MCP** i adminmenuen og opret et token. Tokenet vises kun ved oprettelsen. Brug serveradressen fra siden og Bearer-tokenet i en klient med Streamable HTTP og brugerdefineret Authorization-header.

De 12 MCP-værktøjer giver samme dokument-, proces-, arkiv- og tokenfunktioner som adminfladen. HTML og MCP deler validering og transaktioner. Revisionsudtræk kommer fra én samlet databasetilstand, også når en underskrift registreres samtidig. Modtagernes underskrift kræver stadig deres aktive samtykke. Tokens kan udløbe og tilbagekaldes individuelt; `configure-admin` tilbagekalder alle tokens og sessioner. Se [AI-adgang med MCP](integrations/mcp.md).

## Se et eksempel på bilaget

`cargo run --locked --example receipt_preview` genererer et tydeligt fiktivt eksempel i `var/receipt-preview`, både som samlet PDF og som separat signeringsbilag. Se [bilagets indhold](product/signering-og-arkiv.md#underskrifter-og-dokumentation-i-pdfen) for dokumentreferencer og sammenhængen til revisionssporet.

## Find den rette forklaring

| Opgave | Læs videre |
| --- | --- |
| Forbinde AI eller oprette og tilbagekalde admin-tokens | [AI-adgang med MCP](integrations/mcp.md) |
| Forstå tokens, samtykke og afslutning | [Adgang, signeringsforløb og arkiv](product/signering-og-arkiv.md) |
| Følge processen eller finde en underskrevet aftale | [Procesoverblik og signeret arkiv](product/signering-og-arkiv.md#følg-processen) |
| Starte appen, ændre din konto eller sikre linknøglen | [Lokal drift](operations/lokal-drift.md) |
| Holde Rust-buildfiler og runtime-data adskilt | [Buildplads og kontrol](operations/lokal-drift.md#buildplads-og-kontrol) |
| Opsætte udviklingsmiljø, CI og buildcache | [Udviklingsvejledning](../docs/development.md) |
| Læse kode og afprøve ændringer | [README](../README.md), [webflow](../src/web.rs) og [integrationstests](../tests/workflow.rs) |

Produktnavnet er **Skriv Under**. Den eksisterende projektmappe, Rust-crate og buildcache bruger fortsat den tekniske identifikator `document-signing`.
