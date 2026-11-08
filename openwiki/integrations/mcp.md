---
type: integration
title: AI-adgang med MCP og fuld adminparitet
description: Opret og tilbagekald fulde admin-tokens, forbind en MCP-klient og brug samme dokumenthandlinger som adminfladen.
tags: [mcp, ai, tokens, administration, access-control]
verified:
  - by: openwiki/0.9.0
    at: 2026-11-08T20:04:33.376Z
sources:
  - id: openwiki-source-b6b7bc17569f2d75c652ef4d
    resource: repo://src/admin.rs
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
  - id: openwiki-source-a4d0d6fea18ea7627c71affd
    resource: repo://tests/workflow.rs
generated: { by: "codex", at: "2026-11-08T19:54:14.556Z" }
---

# AI-adgang med MCP

Ejeren kan give en valgt AI-klient samme muligheder som adminfladen. Klienten handler som den eksisterende administrator; den opretter ingen ekstra brugerkonto. Modtagere beholder deres dokumentbundne adgang og eget samtykkeforløb.

## Opret og forbind

Åbn **AI / MCP** i administratorens navigation. Angiv et tokennavn og vælg 7, 30, 90 eller 365 dage. Det fulde token vises kun i svaret ved oprettelsen. Gem det i klientens hemmelige adgangsindstillinger.

Brug adressen vist på siden: som standard `http://127.0.0.1:3080/mcp`. Vælg Streamable HTTP og angiv `Authorization: Bearer DIT_TOKEN`. Klienten skal understøtte en brugerdefineret Authorization-header. OAuth-login og automatisk klientregistrering er ikke implementeret. Offentlig adgang kræver HTTPS og en konfigureret `BASE_URL`.

## Paritet med adminfladen

| Handling | MCP-værktøj |
| --- | --- |
| Søgning, filtrering, 25 rækker pr. side og signeret arkiv | `list_documents` |
| Dokument, besked, underskrivere, status, hændelser og aktuelle links | `get_document` |
| Upload én PDF eller en dokumentpakke og opret kladde | `create_document` |
| Start runde, frist og personlige links | `start_signing` |
| Udskift afventendes links | `rotate_signing_links` |
| Annuller kladde eller igangværende runde | `cancel_document` |
| Udskift læselinks til afsluttet aftale | `renew_download_links` |
| Hent original, færdig PDF eller separat bilag | `get_document_file` |
| Hent revisionsspor med signaturbeviser | `get_audit_trail` |
| Se tokens og seneste 100 AI-hændelser | `list_admin_tokens` |
| Opret fuldt admin-token | `create_admin_token` |
| Tilbagekald et token, også klientens eget | `revoke_admin_token` |

Upload bruger enten `pdf_base64` for én PDF eller `files: [{"name":"Aftale.pdf","pdf_base64":"…"}, …]` for en dokumentpakke. Angiv præcis én af de to former. Pakken indeholder 1–10 filer, tilsammen højst 20 MiB og 200 sider; også den samlede PDF skal være højst 20 MiB. Base64 er standardformatet uden data-URL. `get_document` og `get_audit_trail` returnerer `files` med rækkefølge, navn, SHA-256, sidetal og første side i pakken. `get_document_file` bruger `original` for den samlede original, `final` for den signerede pakke, `receipt` for bilaget og `source-1` til `source-10` for uændrede originalfiler. Samme validering gælder for PDF, titel, besked, underskrivere og frist. Download returnerer en indlejret PDF-resource med base64-`blob` og MIME-type `application/pdf`, samt tekstmetadata med filnavn, byteantal og SHA-256. Adminadgang til arkiverede filer gælder efter udløb af særskilte læselinks.

HTML og MCP kalder de samme operationer i [admin.rs](../../src/admin.rs). Det omfatter søgning, oprettelse, tilstandsændringer, transaktioner, linkrotation, PDF-bytes og revisionsdata. Revisionsudtræk læses i én transaktion, så status, underskrifter, hændelser og hashes kommer fra samme tilstand, også ved samtidig signering. Metadataopslag indlæser ikke PDF-BLOBs. Derfor gælder blandt andet 60 sekunders ventetid ved linkrotation og samme regler om lukkede runder. MCP kan ikke give samtykke eller underskrive på modtagernes vegne. Lokal kontoopsætning og adgangskodeskift foregår fortsat i terminalen.

## Tokens og tilbagekaldelse

`api_tokens` gemmer bruger-ID, navn, SHA-256, kort visningspræfiks, udløb, oprettelse, seneste brug og eventuel tilbagekaldelse. Det komplette rå token gemmes ikke. Hver forespørgsel kontrollerer, at tokenet ikke er udløbet eller tilbagekaldt. Browsercookies og tokenværdier i URL'en accepteres ikke som MCP-adgang.

Nye tokens kræver en stadig gyldig session eller et stadig gyldigt udstedertoken i selve oprettelsestransaktionen. Det lukker et tidsvindue ved samtidig logout, adgangskodeskift eller tilbagekaldelse.

Alle tokens har fuld adminadgang, herunder mulighed for at oprette andre tokens. Tilbagekaldelse gælder det valgte token; andre tokens skal tilbagekaldes separat. `configure-admin` tilbagekalder alle MCP-tokens og login-sessioner, mens konto-ID og dokumenter bevares. Et igangværende almindeligt værktøjskald kan allerede være udført ved tilbagekaldelse.

## Sporbarhed og HTTP

Adminfladen og `list_admin_tokens` viser de seneste 100 hændelser. `admin_access_events` gemmer token-ID, værktøjsnavn og resultat uden at gemme argumenter, rå credentials, links eller PDF-indhold. Dokumentændringer angiver `mcp:{token-ID}` som aktør i dokumentets revisionsspor.

Endpointet understøtter `initialize`, notifikationer, `ping`, `tools/list` og `tools/call`. Det er stateless og opretter ingen MCP-server-session. POST returnerer JSON; autentificeret GET og DELETE returnerer 405. Protokolversionerne er `2025-11-25`, `2025-06-18` og `2025-03-26`.

Adgangskontrollen kører før request-body læses. Almindelige formularer er begrænset til 64 KiB, multipart-upload til 22 MiB og MCP til 30 MiB; pakken og dens samlede PDF er stadig højst 20 MiB.

`Host` skal matche `BASE_URL`, og en eventuel `Origin` skal matche samme oprindelse. Reverse proxy skal bevare Host. Log ikke Authorization-headeren eller rå links. Ugyldige eller udløbne credentials giver 401; ugyldig Origin eller Host giver 403. JSON-RPC-fejl og værktøjsfejl holdes adskilt. Notifikationer kan aldrig udføre adminhandlinger.

## Verifikation og kode

[MCP-integrationstestene](../../tests/workflow.rs) kontrollerer engangsvisning, CSRF, adgang, Origin/Host, protokol, udløb, tilbagekaldelse, oprettelse af nye tokens og samme dokumentforløb som HTML. Workflowtesten sammenligner de faktiske PDF-bytes og revisionsdata mellem adgangsformerne. Adversarial-testene kontrollerer desuden tidlig afvisning uden at læse body, grænser på formularer, tokenroller og konsistente revisionsdata under sidste underskrift.

Se [MCP-handleren](../../src/mcp.rs), [fælles adminlogik](../../src/admin.rs), [tokensiden](../../src/web.rs), [datamodellen](../../src/schema.sql), [signeringsforløbet](../product/signering-og-arkiv.md) og [lokal drift](../operations/lokal-drift.md).
