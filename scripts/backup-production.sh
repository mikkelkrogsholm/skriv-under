#!/usr/bin/env bash
# Installed at /usr/local/sbin/skriv-under-backup. Run as root via systemd.
set -euo pipefail
umask 077
runuser -u admin -- /usr/bin/mountpoint -q /home/admin/NAS || { echo "NAS mount is unavailable; off-server backup aborted" >&2; exit 1; }
exec 9>/run/lock/skriv-under-backup.lock
flock -n 9 || exit 0
backup_root=/var/backups/skriv-under
stamp=$(date -u +%Y%m%dT%H%M%SZ)
snapshot="$backup_root/$stamp"
install -d -m 0750 -o root -g admin "$backup_root"
install -d -m 0700 -o admin -g admin "$snapshot"
python3 - "$snapshot" <<'PY'
import sqlite3, pathlib, shutil, sys, os, json, datetime
snapshot = pathlib.Path(sys.argv[1])
source = pathlib.Path('/var/lib/skriv-under')
with sqlite3.connect('file:' + str(source/'documents.sqlite3') + '?mode=ro', uri=True) as src:
    with sqlite3.connect(snapshot/'documents.sqlite3') as dst:
        src.backup(dst)
        if dst.execute('PRAGMA integrity_check').fetchone()[0] != 'ok':
            raise RuntimeError('SQLite backup integrity check failed')
key = (source/'link-key').read_bytes()
if len(key) != 32:
    raise RuntimeError('Invalid production link key')
(snapshot/'link-key').write_bytes(key)
(snapshot/'manifest.json').write_text(json.dumps({
    'created_at': datetime.datetime.now(datetime.timezone.utc).isoformat(),
    'release': str(pathlib.Path('/opt/skriv-under/current').resolve()),
    'base_url': 'https://skrivunder.mikkelkrogsholm.dk',
}, indent=2) + '\n')
for file in snapshot.iterdir():
    os.chmod(file, 0o600)
    shutil.chown(file, user='admin', group='admin')
PY
# SSHFS belongs to admin. Do not alter or reuse other services' backup repositories.
repo=/home/admin/NAS/skriv-under/restic
password=/home/admin/.config/skriv-under/restic-password
runuser -u admin -- /usr/bin/restic --repo "$repo" --password-file "$password" backup "$snapshot" --tag skriv-under --host skriv-under --quiet
runuser -u admin -- /usr/bin/restic --repo "$repo" --password-file "$password" forget --tag skriv-under --host skriv-under --keep-daily 14 --keep-weekly 8 --keep-monthly 12 --quiet
# Retain the latest seven local snapshots, after the off-server backup succeeds.
python3 - "$backup_root" <<'PY'
import pathlib, re, shutil, sys
root = pathlib.Path(sys.argv[1])
backups = sorted(p for p in root.iterdir() if p.is_dir() and re.fullmatch(r'\d{8}T\d{6}Z',p.name))
for old in backups[:-7]:
    shutil.rmtree(old)
PY
printf 'Skriv Under backup verified locally and copied encrypted to NAS: %s\n' "$stamp"
