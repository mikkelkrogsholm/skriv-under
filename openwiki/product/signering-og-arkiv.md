---
type: workflow
title: Adgang, signeringsforløb og arkiv
description: Skriv Unders adgangsmodel, manuelt delte signeringslinks, procesoverblik, arkiv og PDF-bilag med konkrete underskrifter og dokumentation.
tags: [signing, access-control, tokens, workflow, archive, audit]
verified:
  - by: openwiki/0.9.0
    at: 2026-11-07T11:19:15.922Z
sources:
  - id: openwiki-source-abe5fae4c5d81d8288df4316
    resource: repo://examples/receipt_preview.rs
  - id: openwiki-source-b6b7bc17569f2d75c652ef4d
    resource: repo://src/admin.rs
  - id: openwiki-source-1e35ba9c5eb8fd2aeed68a14
    resource: repo://src/db.rs
  - id: openwiki-source-2d54f64140ade8c180921259
    resource: repo://src/links.rs
  - id: openwiki-source-b95ec25c5cfc0be350df258c
    resource: repo://src/mcp.rs
  - id: openwiki-source-efa620b89ed22813d5a01dbf
    resource: repo://src/pdf.rs
  - id: openwiki-source-ed3a7abb0422ee495e36f6ad
    resource: repo://src/process.js
  - id: openwiki-source-10dbc7a48ac0283046204658
    resource: repo://src/schema.sql
  - id: openwiki-source-7b75006986d8b0337adc6ca0
    resource: repo://src/web.rs
  - id: openwiki-source-a4d0d6fea18ea7627c71affd
    resource: repo://tests/workflow.rs
generated: { by: "codex", at: "2026-11-07T11:19:15.922Z" }
---

# Adgang, signeringsforløb og arkiv

Skriv Under har ejerens administratorlogin, fulde admin-tokens til ejerens AI-klienter og modtagernes personlige dokumentlinks. Produktet viser samme navn i browseren og nye signeringskvitteringer.

## Én administrator, dokumentbundne modtagere

Databasen tillader højst én række i `users` gennem et unikt indeks. Der er ingen offentlig registrering og ingen modtagerkonti. Administratoren kan oprette dokumenter, oprette personlige links, følge processen og bruge arkivet. HTML kontrollerer login-session og CSRF ved ændringer. MCP kontrollerer et separat bearer-token med udløb og tilbagekaldelse. Begge handler som samme ejer og kontrollerer dokumentejerskab.

Modtageren får et personligt link med et 256-bit token, der udledes fra en privat nøgle og en tilfældig nonce. Modtagerrækken gemmer SHA-256-hashen, og tokenet peger på én modtager i ét bestemt dokument. Modtagersiderne udleder dokument-ID'et fra den række. Et modtagertoken giver ingen adgang til administratorens lister, status-endpoint, revisions-eksport eller arkiv.

En invitation åbnes via `/s/{token}`, lægges i en HttpOnly-cookie og viderestilles til `/sign`. Åbningen registreres med sammenlægning af gentagelser, men forbruger ikke tokenet og underskriver ikke dokumentet. Et link kan være åbnet automatisk; det er ikke bevis for, at et menneske har læst indholdet.

## AI med samme administratoradgang

Ejeren kan oprette MCP-tokens på **AI / MCP**. De giver 1:1 paritet med adminfladen: dokumentoprettelse, søgning, links, annullering, proces, arkiv, PDF'er, revisionsspor og tokenstyring. De fælles handlinger i [admin.rs](../../src/admin.rs) bevarer samme validering og transaktioner. En AI kan ikke give modtagerens samtykke eller registrere underskrift gennem et adminværktøj.

AI-ændringer kan identificeres med `mcp:{token-ID}` i revisionssporet. Tokenudløb, seneste brug, individuelle tilbagekaldelser og de seneste AI-hændelser er synlige i admin. Se [forbindelse og tokenlivscyklus](../integrations/mcp.md).

## Fra kladde til underskrift

Administratoren uploader en PDF på højst 20 MiB og 200 sider og angiver titel, besked, 1–20 unikke underskrivernavne og en frist på 7, 14 eller 30 dage. PDF'er med blandt andet adgangskodebeskyttelse, aktivt indhold, vedhæftede filer eller eksisterende certifikatunderskrifter afvises. Kontrollen dækker også FileAttachment-annoteringer og EF-filer uden navnetræ. Sidetræet må bruge indirekte Kids og Count; sideantallet skal stemme med de faktiske sider.

Originale PDF-bytes og deres SHA-256-hash gemmes. Dokumentet og modtagerne kan ikke redigeres efter upload; en ændring kræver en ny runde. Ved **Opret signeringslinks** går kladden til `pending`, fristen starter, og administratoren får et link pr. underskriver. Der kræves ingen modtager-e-mail. Administratoren kopierer og deler selv links gennem sin valgte kanal.

| Dokumentstatus | Betydning |
| --- | --- |
| `draft` | Kladde, endnu ingen signeringslinks |
| `pending` | Personlige links er klar til deling; underskrifter afventes |
| `completed` | Alle har underskrevet, og færdige PDF'er er gemt |
| `declined` | En modtager har afvist; runden er lukket |
| `expired` | Fristen er udløbet |
| `cancelled` | Administratoren har annulleret runden |

Modtageren indtaster sit navn og accepterer den viste samtykketekst, før POST-handlingen kan registrere underskriften. CSRF, dokumentstatus, udløb, token og tidligere svar kontrolleres. Opdateringen og revisionshændelsen sker i én transaktion. En allerede besvaret invitation kan ikke underskrive igen.

**Erstat links til afventende** roterer tokens for ubesvarede invitationer og gør deres tidligere links ugyldige. Allerede registrerede underskrifter bevares. Der skal gå mindst 60 sekunder mellem linkudskiftninger. Links kan kopieres igen fra administratorens dokumentside, også efter genstart. Afvisning med begrundelse og annullering lukker runden; udløb kontrolleres både ved indsendelse og i vedligeholdelsesarbejderen.

## Følg processen

Dokumentlisten har søgning på titel og navn, statusfiltre, sideinddeling og antal underskrifter. Dokumentets detaljeside viser:

- Antal underskrifter og navnene på dem, der mangler.
- Personlige links med kopieringsknap og tidspunktet for linkoprettelse.
- Seneste registrerede linkåbning og PDF-adgang med forbehold for automatiske besøg.
- Registrerede underskrifter, tidspunkter og eventuelle afvisningsgrunde.
- En tidslinje på dansk og mulighed for at eksportere revisionssporet som JSON.

Mens en igangværende runde er synlig, spørger browseren det administratorbeskyttede status-endpoint hvert 10. sekund. Nye hændelser eller ændret status genindlæser siden. Opdateringen stopper, når runden er afsluttet eller login er udløbet. Linkoprettelse er ikke bevis for faktisk deling, og appen registrerer ingen maillevering.

Gentagne linkåbninger og PDF-visninger registreres højst én gang pr. modtager og hændelsestype pr. minut. Nye besøg efter intervallet tilføjer nye uændrede hændelser. Tidspunkterne er derfor registrerede adgangshændelser, ikke en komplet besøgslog.

## Afslutning og signeret arkiv

Den sidste underskrift udløser dannelse af en særskilt kvitterings-PDF og en færdig PDF med originalens sider plus kvitteringssider. Dokumentstatus, filer, slutfilens hash og afslutningshændelsen gemmes sammen med underskriften. Fejl ved PDF-dannelsen betyder, at den transaktion ikke gemmes.

Administratoren får læselinks til den færdige PDF og kvittering. De kan kopieres og deles manuelt, udløber efter 30 dage og kan erstattes med nye. Udskiftning tilbagekalder det tidligere sæt læselinks. Modtagere kan også vende tilbage via deres oprindelige signeringslink, når alle har underskrevet. Systemet udsender ingen automatisk besked om afslutning eller afvisning.

Administratorsiden `/archive` bruger login-sessionen og har ingen dokumentlinkfrist. Afsluttede aftaler vises automatisk med nyeste afslutning først og kan søges frem og hentes efter genstart, også når de særskilte læselinks er udløbet.

PDF'erne indeholder en dokumenteret elektronisk bekræftelse. Der foretages ingen MitID-kontrol, og kvitteringen er ikke en certifikatbaseret PDF-signatur eller et uafhængigt tidsstempel. Kvitteringens font understøtter Windows-1252, herunder danske tegn; øvrige navnetegn bevares som Unicode i databasen og JSON-eksporten.


## Underskrifter og dokumentation i PDF'en

Nye afsluttede aftaler får bilaget **Underskrifter og dokumentation** efter originalens sider. Det samme bilag kan hentes som en separat PDF. Eksisterende arkiverede aftaler omskrives ikke.

Bilaget viser dokumentreference `SU-{dokument-ID}`, titel, originalens sideantal, SHA-256 af originalen uden bilag og tidspunktet for den sidste registrerede underskrift. Hver underskriver får en reference `SU-{dokument-ID}-{modtager-ID}` samt:

- Det indtastede underskriftsnavn, fremhævet som elektronisk underskrift.
- Det navn administratoren havde tildelt invitationen.
- Serverregistreret tidspunkt i UTC med sekunder.
- Den konkrete godkendelseshandling og den faktisk gemte samtykketekst for personen.

Metoden beskrives som et personligt link oprettet til manuel deling. Bilaget indeholder ingen tokens og påstår ikke, at linket faktisk blev sendt, at personen er identitetskontrolleret, eller at en certifikatsignatur er anvendt.

Ved mange underskrivere eller lange navne fortsætter bilaget over flere nummererede sider. Underskriftsblokke holdes samlet, når de kan være på én side. Bilagssider har deres egen A4-beskæring og rotation, også hvis originalen er roteret eller beskåret.

Eksportens status, underskrifter, hash og hændelser læses i én databasetransaktion, så en samtidig underskrift ikke blander to tilstande. Metadataopslag indlæser kun dokumentfelter, ikke PDF-BLOBs.

JSON-eksporten indeholder samme dokumentreference og en liste med signeringsreferencer, modtagernavne, underskrevne navne, gemte samtykker og signeringstidspunkter som Unix-sekunder. Hændelserne får desuden deres oprindelige numeriske tidsstempel, så tider ikke kun fremgår afrundet til minutter.

Et tydeligt fiktivt eksempel kan genereres med `cargo run --locked --example receipt_preview`. Det gemmer en samlet PDF og et separat bilag i `var/receipt-preview`. Eksempelkoden ligger i [receipt_preview.rs](../../examples/receipt_preview.rs).

## Kode og verifikation

Adgang og modtagersignering ligger i [webhandlerne](../../src/web.rs), fælles administratorhandlinger i [adminlogikken](../../src/admin.rs) og AI-adgang i [MCP-handleren](../../src/mcp.rs), persistens i [databasen](../../src/db.rs) og [skemaet](../../src/schema.sql), procesopdatering i [browserkoden](../../src/process.js), linkgenerering i [linkmodulet](../../src/links.rs) og kvitteringer i [PDF-modulet](../../src/pdf.rs).

[Integrationstestene](../../tests/workflow.rs) efterprøver blandt andet én administrator, dokumentbundne tokens, samtykke, CSRF, samtidige gentagelser, linkrotation, genkopiering efter genstart, databaseopgradering, udløb, procesoverblik og arkivadgang efter genstart og downloadudløb.

Se [kom i gang](../quickstart.md) og [lokal drift](../operations/lokal-drift.md).
