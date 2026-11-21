# Installation and distribution

**Available only after GitHub Releases completes and (for npx) npm registry publishing completes.** These are packaging contracts, not a claim that binaries are already downloadable.

## Linux and macOS

After the first GitHub release:
```sh
curl -fsSLO https://raw.githubusercontent.com/KeyserDSoze/RystemDatabase/main/scripts/install.sh
bash ./install.sh
rystemdb --version
```

Use `RYSTEMDB_VERSION=v0.1.1` to select a specific published version, `RYSTEMDB_INSTALL_DIR` to change the destination. Installer checks SHA-256 and puts the binary in `~/.local/bin` by default; add that folder to PATH.

## Windows (PowerShell)

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/KeyserDSoze/RystemDatabase/main/scripts/install.ps1 -OutFile install-rystemdb.ps1
powershell -NoProfile -File .\install-rystemdb.ps1
rystemdb --version
```

The user PATH is updated for subsequent terminals. For a pinned release pass `-Version v0.1.1`.

## npm / npx

After npm registration/publication:
```sh
npx --yes rystemdb --version
npx --yes rystemdb serve
```

Node.js 20+ required. The npm wrapper downloads a version-matched native release file and validates its SHA256SUMS manifest. The binary is not a JavaScript reimplementation.

## Docker

```sh
docker compose -f docker/compose.yml up --build
```

Published images (after version-tag workflow): `ghcr.io/keyserdsoze/rystemdatabase:v0.1.1`.

Current CLI only exposes health endpoints; it **does not persist data** and is not a production service. The Docker image does not have a persistent data volume for this reason.
