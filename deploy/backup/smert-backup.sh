#!/usr/bin/env bash
# Nightly backup of smert's data volume to the Storage Box with restic.
# Installed on the server as /usr/local/bin/smert-backup and run by
# smert-backup.timer. Settings come from /etc/smert-backup/env.
set -euo pipefail

source /etc/smert-backup/env
export RESTIC_REPOSITORY RESTIC_PASSWORD_FILE

# healthchecks.io: /start when beginning, the bare URL on success, /fail with
# the end of the log on any error. A missed night alerts by itself.
ping() { curl -fsS -m 10 --retry 3 -o /dev/null "$@" || true; }
log=$(mktemp)
trap 'status=$?
      if [ $status -ne 0 ]; then tail -c 10000 "$log" | ping --data-binary @- "$HC_PING_URL/fail"; fi
      rm -f "$log"' EXIT
exec > >(tee -a "$log") 2>&1

ping "$HC_PING_URL/start"

# A run that crashed can leave its lock behind, which would block the
# cleanup below every night after. This only removes locks whose process
# is gone.
restic unlock

# Stage a data folder (smert.db + images/) so a restore gives back exactly
# what deploy/seed.sh expects.
stage=/var/cache/smert-backup
rm -rf "$stage" && mkdir -p "$stage"

# The database is copied with SQLite's backup API while the app keeps
# running; copying the file itself could catch a half-finished write.
python3 - "$VOLUME_DIR/smert.db" "$stage/smert.db" <<'PY'
import sqlite3, sys
src = sqlite3.connect(f"file:{sys.argv[1]}?mode=ro", uri=True)
dst = sqlite3.connect(sys.argv[2])
src.backup(dst)
assert dst.execute("PRAGMA integrity_check").fetchone()[0] == "ok"
dst.close()
PY

# Images never change once written, so hard links stand in for copies.
# Making them bumps each file's ctime, hence --ignore-ctime below.
cp -al "$VOLUME_DIR/images" "$stage/images"

cd "$stage"
restic backup --host smert --ignore-ctime smert.db images

# Keep only the latest snapshot and drop data nothing references any more.
restic forget --host smert --keep-last 1 --prune
restic check

rm -rf "$stage"
ping "$HC_PING_URL"
echo "Backup finished."
