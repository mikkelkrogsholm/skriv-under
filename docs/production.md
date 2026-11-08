# Produktion på skrivunder.mikkelkrogsholm.dk

Skriv Under kører på https://skrivunder.mikkelkrogsholm.dk med MCP på `/mcp`.
Caddy håndterer HTTPS og videresender til appen på `127.0.0.1:3080`.
Admin hedder `mikkel`; den oprindelige adgangskode ligger privat på serveren i
`/home/admin/.config/skriv-under/initial-admin.txt` (0600). Den må ikke lægges i Git.

## Installation og isolation

- Dedikeret systembruger: `skriv-under`, uden interaktivt login.
- Binære releases: `/opt/skriv-under/releases/<dato>-<binary-sha256-prefix>/document-signing`.
- Aktiv release: `/opt/skriv-under/current`, et symlink til et releasebibliotek.
- Privat konfiguration: `/etc/skriv-under/production.env` (root, 0600).
- Data: `/var/lib/skriv-under` (0700), inklusive `documents.sqlite3` og `link-key`.
- App-service: `/etc/systemd/system/skriv-under.service`.
- Caddy-site: `/etc/caddy/SkrivUnder.Caddyfile`, importeret fra den eksisterende Caddyfile.

Skabelonerne ligger i [deploy](../deploy/production.env.example). Appen kører som
sin egen bruger med skriveadgang til sin datamappe, hukommelsesgrænse på 1 GiB,
CPU-grænse på to kerner og automatisk genstart ved fejl. Den lytter kun lokalt.
`TRUST_PROXY=true` aktiverer klient-IP fra Caddys overskrevne
`X-Skriv-Under-Client-IP`; offentligt bind med denne indstilling afvises ved opstart.

Caddy-konfigurationen tilføjer kun dette site. Den eksisterende fil har fået
`import /etc/caddy/SkrivUnder.Caddyfile` til sidst. Ved ændringer valideres den
samlede konfiguration, og Caddy genindlæses uden genstart:

```sh
sudo -u caddy caddy validate --config /etc/caddy/Caddyfile --adapter caddyfile
sudo systemctl reload caddy
```

Sitet har ingen accesslog. En generisk fejlside håndterer også en utilgængelig
backend, så tokenstier og request-headers ikke kommer i Caddys normale fejllog.
Aktivér ikke debuglogning af requests: links, cookies og Authorization er hemmeligheder.

## Kontrol og opdatering

```sh
curl -fsS https://skrivunder.mikkelkrogsholm.dk/health
sudo systemctl status skriv-under.service
sudo journalctl -u skriv-under.service -n 30 --no-pager
sudo systemctl status skriv-under-backup.timer
```

Byg en release fra et gennemgået checkout med `cargo build --release --locked -j 2`.
Buildcache bliver uden for checkoutet; produktionsbinæren kopieres ud af cachen,
så oprydning af Rust-builds ikke påvirker driften. Installér binæren root-ejet og
0755 i et nyt releasebibliotek, kontrollér SHA-256, og tag backup før opdatering.
Skift `current` til det nye bibliotek, og genstart kun `skriv-under.service`.
Kontrollér `/health`, login og eksisterende dokumenter efter opdateringen.
Ved tilbagerulning skal eventuelle databaseskemaændringer også vurderes; en gammel
binær er ikke nødvendigvis kompatibel med en nyere database.

Administratorens adgangskode ændres interaktivt på serveren:

```sh
sudo -u skriv-under env DATA_DIR=/var/lib/skriv-under \
  /opt/skriv-under/current/document-signing configure-admin
```

Dette tilbagekalder alle login-sessioner og MCP-tokens. Opdatér derefter den
private adgangskodenote. Der findes ingen offentlig kontooprettelse eller nulstilling.

## Backup

`skriv-under-backup.timer` kører hver dag kl. 03:35–03:40 dansk tid og indhenter
forpassede kørsler efter opstart. Scriptet [backup-production.sh](../scripts/backup-production.sh)
er installeret som `/usr/local/sbin/skriv-under-backup` og gør følgende:

1. Kontrollerer at NAS faktisk er monteret, set fra brugeren `admin`, som ejer SSHFS-mountet.
2. Tager SQLite online-backup og kontrollerer `PRAGMA integrity_check`.
3. Gemmer linknøglen og et manifest med tidspunkt, release og BASE_URL i samme snapshot.
4. Sender snapshot til et særskilt krypteret restic-repository på NAS.

Lokale snapshots ligger i `/var/backups/skriv-under`; de seneste syv beholdes
efter vellykket NAS-backup. NAS-repository er `/home/admin/NAS/skriv-under/restic`.
Restic beholder 14 daglige, otte ugentlige og 12 månedlige snapshots. `forget`
fjerner gamle snapshotreferencer; frigivelse af urefererede datablokke kræver
senere `restic prune` i netop dette repository.

Krypteringsnøglen ligger i `/home/admin/.config/skriv-under/restic-password`
(0600). Gem en separat sikker kopi: NAS-backuppen kan ikke gendannes uden den.
Andre services' backupjobs og repositories bruges ikke.

```sh
sudo systemctl start skriv-under-backup.service
sudo systemctl show skriv-under-backup.service -p Result -p ExecMainStatus
restic --repo /home/admin/NAS/skriv-under/restic \
  --password-file /home/admin/.config/skriv-under/restic-password snapshots
```

En manglende NAS eller en fejl ved kryptering/overførsel giver en fejlet service;
en lokal kopi alene tæller ikke som en vellykket backup.

## Gendannelse

Kør restic som `admin`, der har adgang til NAS-mountet. Vælg det ønskede snapshot
med `snapshots`, og gendan først til en ny, privat mappe:

```sh
mkdir -m 700 /home/admin/skriv-under-restore
restic --repo /home/admin/NAS/skriv-under/restic \
  --password-file /home/admin/.config/skriv-under/restic-password \
  restore SNAPSHOT_ID --target /home/admin/skriv-under-restore
```

Find `documents.sqlite3`, `link-key` og manifestet under den gendannede sti.
Kontrollér databaseintegritet og matchende release. Start eventuelt den gendannede
app isoleret med en ny DATA_DIR og en ledig loopback-port. Brug aldrig samme
DATA_DIR fra to processer. Bevar database og linknøgle sammen; nøglen er nødvendig
for eksisterende links.

Ved faktisk gendannelse stoppes kun `skriv-under.service`. Gem den nuværende
datamappe som en privat sikkerhedskopi, og installér det gendannede datasæt i en
frisk `/var/lib/skriv-under`, ejet af `skriv-under`, med mappe 0700 og filer 0600.
Kopiér ikke gamle WAL/SHM-filer oven i en restic-gendannet SQLite online-backup.
Kør `configure-admin` for at tilbagekalde sessioner og MCP-tokens fra snapshotet,
start servicen, og kontrollér HTTPS, login, arkiv og download af en kendt aftale.

Ved idriftsættelsen blev et snapshot med en fiktiv underskrevet aftale gendannet
fra det krypterede NAS-repository. SQLite-integritet, linknøgle, PDF-hash og opstart
på en særskilt lokal port blev kontrolleret. Testdokumenterne blev derefter fjernet
fra den aktive installation.
