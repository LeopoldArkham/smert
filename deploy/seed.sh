#!/usr/bin/env bash
# Copy a local data folder (database + images) into smert's Coolify volume.
# Run from the repo root on your Mac:
#
#   deploy/seed.sh root@<server> <volume name> [local data dir, default ./data]
#
# The volume name is shown in Coolify under the app's Persistent Storage.
# Stops the app while copying and starts it again afterwards. Refuses if the
# server's database already has content.
set -euo pipefail

if [ $# -lt 2 ]; then
  echo "usage: $0 root@<server> <volume name> [data dir]" >&2
  exit 2
fi
host=$1
volume=$2
data=${3:-data}

if [ ! -f "$data/smert.db" ]; then
  echo "No $data/smert.db: expected a smert data folder (smert.db + images/)." >&2
  exit 1
fi

mount=$(ssh "$host" docker volume inspect --format '{{.Mountpoint}}' "$volume")

remote_rows=$(ssh "$host" python3 - "$mount/smert.db" <<'PY'
import os, sqlite3, sys
db = sys.argv[1]
if not os.path.exists(db):
    print(0)
else:
    tables = ["quotes", "artworks", "definitions", "scriptures"]
    con = sqlite3.connect(db)
    print(sum(con.execute(f"SELECT COUNT(*) FROM {t}").fetchone()[0] for t in tables))
PY
)
if [ "$remote_rows" != "0" ]; then
  echo "The server's database already has $remote_rows items; not overwriting it." >&2
  exit 1
fi

# A consistent copy, even if the app is running locally.
tmp=$(mktemp -d)
sqlite3 "$data/smert.db" ".backup '$tmp/smert.db'"

containers=$(ssh "$host" docker ps --quiet --filter "volume=$volume")
if [ -n "$containers" ]; then
  echo "Stopping the app while copying…"
  ssh "$host" docker stop $containers >/dev/null
fi

ssh "$host" "rm -f '$mount/smert.db' '$mount/smert.db-wal' '$mount/smert.db-shm'"
echo "Copying images ($(du -sh "$data/images" | cut -f1))…"
# Without these, macOS tar adds ._* metadata files and extended attributes.
COPYFILE_DISABLE=1 tar --no-xattrs --no-mac-metadata -C "$data" -cf - images \
  | ssh "$host" "tar -C '$mount' -xf -"
scp -q "$tmp/smert.db" "$host:$mount/smert.db"
rm -rf "$tmp"
# 10001 is the smert user inside the image (see Dockerfile).
ssh "$host" "chown -R 10001:10001 '$mount'"

if [ -n "$containers" ]; then
  ssh "$host" docker start $containers >/dev/null
  echo "App restarted."
fi
echo "Seeded."
