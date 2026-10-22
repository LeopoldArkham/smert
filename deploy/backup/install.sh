#!/usr/bin/env bash
# Install or update the backup job on the server. Run from the repo root:
#
#   deploy/backup/install.sh root@<server>
#
# Safe to re-run: it never overwrites the settings, the encryption password
# or the SSH key once they exist, and it doesn't enable the timer (see
# deploy/backup/README.md for the remaining one-time steps).
set -euo pipefail

if [ $# -ne 1 ]; then
  echo "usage: $0 root@<server>" >&2
  exit 2
fi
host=$1
dir=$(dirname "$0")

scp -q "$dir/smert-backup.sh" "$dir/smert-backup.service" "$dir/smert-backup.timer" \
       "$dir/env.example" "$host:/tmp/"

ssh "$host" 'bash -s' <<'REMOTE'
set -euo pipefail
export DEBIAN_FRONTEND=noninteractive
command -v restic >/dev/null || { apt-get update -q && apt-get install -y -q restic; }

install -m 755 /tmp/smert-backup.sh /usr/local/bin/smert-backup
install -m 644 /tmp/smert-backup.service /tmp/smert-backup.timer /etc/systemd/system/
systemctl daemon-reload

install -d -m 700 /etc/smert-backup
[ -f /etc/smert-backup/env ] || install -m 600 /tmp/env.example /etc/smert-backup/env
if [ ! -f /etc/smert-backup/restic-password ]; then
  (umask 077 && openssl rand -base64 32 > /etc/smert-backup/restic-password)
  echo "Generated a new restic encryption password in /etc/smert-backup/restic-password."
fi

install -d -m 700 /root/.ssh
[ -f /root/.ssh/smert_backup ] || ssh-keygen -q -t ed25519 -N "" -C "smert-backup@$(hostname)" -f /root/.ssh/smert_backup

rm -f /tmp/smert-backup.sh /tmp/smert-backup.service /tmp/smert-backup.timer /tmp/env.example
echo "restic $(restic version | cut -d' ' -f2) installed."
echo "Public key to add to the Storage Box:"
cat /root/.ssh/smert_backup.pub
REMOTE
