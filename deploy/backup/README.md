# Backups

Every night at 03:30 (Paris time) the server backs up smert's data volume
(`smert.db` + `images/`) to a Hetzner Storage Box with
[restic](https://restic.net). Backups are encrypted and only the latest one is
kept. healthchecks.io emails you if a night's backup fails or doesn't run.

On the server:

| What | Where |
|---|---|
| Backup script | `/usr/local/bin/smert-backup` (from `smert-backup.sh`) |
| Timer | `smert-backup.timer` → `smert-backup.service` |
| Settings | `/etc/smert-backup/env` (from `env.example`) |
| Encryption password | `/etc/smert-backup/restic-password` |
| SSH key for the Storage Box | `/root/.ssh/smert_backup` |

`install.sh` puts these in place and is safe to re-run after editing the
script or units.

## One-time setup

1. **Order a Storage Box** in the Hetzner Console (the smallest is plenty).
   Prefer a different location from the server (e.g. Nuremberg or Helsinki),
   so one datacenter problem can't take out both. Enable **SSH support**, and
   add this public key:

   ```
   ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAIJKJRn/a464yKgwXNZ7utDvWsMMsQk9XEG+daGBYhBD+ smert-backup@server-1
   ```

2. **Create a healthchecks.io check**: period 1 day, grace time 2 hours. Put
   its ping URL in `HC_PING_URL` in `/etc/smert-backup/env` on the server.

3. **Save the encryption password** in your password manager:

   ```bash
   ssh root@167.233.56.221 cat /etc/smert-backup/restic-password
   ```

   Without it the backups can't be decrypted, e.g. after losing the server.

4. **Connect, initialise and test** (on the server; `uNNNNNN` is the Storage
   Box username):

   ```bash
   cat >> /root/.ssh/config <<'EOF'
   Host storagebox
       HostName uNNNNNN.your-storagebox.de
       User uNNNNNN
       Port 23
       IdentityFile /root/.ssh/smert_backup
   EOF
   ssh-keyscan -p 23 uNNNNNN.your-storagebox.de >> /root/.ssh/known_hosts
   set -a; source /etc/smert-backup/env; set +a
   restic init
   smert-backup
   systemctl enable --now smert-backup.timer
   ```

## Day to day

- **Last runs**: `journalctl -u smert-backup`
- **Next run**: `systemctl list-timers smert-backup.timer`
- **Run now**: `systemctl start smert-backup`
- **What's stored**: `set -a; source /etc/smert-backup/env; set +a; restic snapshots`

## Restoring

On the server, restore the latest snapshot into a folder:

```bash
set -a; source /etc/smert-backup/env; set +a
restic restore latest --target /root/smert-restore
```

`/root/smert-restore` is then a data folder (`smert.db` + `images/`). To put
it back live, stop the app in Coolify, replace the volume's contents with it
(the volume is in `VOLUME_DIR`), run
`chown -R 10001:10001 "$VOLUME_DIR"`, and start the app.

On a new server, restic only needs the Storage Box access and the encryption
password from your password manager; `deploy/seed.sh` can then copy the
restored folder into the new app's volume.
