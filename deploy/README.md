# Deploying smert

smert runs on the Hetzner server (`167.233.56.221`) through **Coolify**, whose
dashboard is at `http://167.233.56.221:8000`. Coolify builds the `Dockerfile`
from GitHub, runs the container, and routes web traffic to it through its
Traefik proxy. The database and images live in a Docker volume mounted at
`/data`, which survives redeploys.

GitHub Actions (`.github/workflows/ci.yml`) runs the tests and builds the
Docker image on every push, so a broken build shows up there.

## One-time setup

### 1. Create the app in Coolify

In the dashboard: **Projects → your project → + New → Public Repository**.

- Repository URL: `https://github.com/LeopoldArkham/smert`, branch `master`
- Build Pack: **Dockerfile**

Then, in the app's settings:

- **General → Ports Exposes**: `1212`
- **General → Domains**: keep the generated `http://….167.233.56.221.sslip.io`
  address for now; replace it with your own domain later (Coolify then gets
  HTTPS certificates by itself).
- **Persistent Storage → + Add → Volume**: destination path `/data`. Note the
  volume name Coolify shows; step 4 needs it.
- Leave the health check off: the image has no `curl` for Coolify to run it.

Click **Deploy**. The first build takes several minutes; later ones reuse the
cached dependencies.

### 2. Deploy on every push

Public repositories don't get automatic deploys. In the app's **Webhooks**
section, copy the GitHub webhook URL and set a secret, then add them in GitHub
under **Settings → Webhooks → Add webhook** (content type `application/json`,
"Just the push event").

### 3. SSH access for copying data

Your Mac needs SSH access as root. Create a key if you don't have one, then
add it on the server, e.g. from Coolify's **Servers → (server) → Terminal**:

```bash
ssh-keygen -t ed25519
cat ~/.ssh/id_ed25519.pub
```

On the server, append that line to `/root/.ssh/authorized_keys`.

### 4. Copy your data, once

```bash
cargo run -- import
deploy/seed.sh root@167.233.56.221 VOLUME_NAME
```

Skip the import if `data/smert.db` already exists. `seed.sh` stops the app,
copies the database and images into the volume, and starts it again. It
refuses if the server's database already has content.

## Day to day

- **Logs, restarts, rollbacks**: the app's page in Coolify (Deployments lists
  previous builds; any of them can be redeployed).
- **Settings**: `SMERT_DATA_DIR` and `SMERT_BIND` are set in the `Dockerfile`;
  override them in Coolify's Environment Variables if needed.
