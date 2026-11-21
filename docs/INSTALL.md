# Installation and distribution

Install native release binaries only after verifying the version exists in [GitHub Releases](https://github.com/KeyserDSoze/RystemDatabase/releases). `v0.1.2` is health-only; the first document CRUD CLI preview is planned for `v0.1.2`. The npm/npx wrapper remains unavailable until actual npm registry publication.

## Linux and macOS

For a published native release (pin the required version if needed):
```sh
curl -fsSLO https://raw.githubusercontent.com/KeyserDSoze/RystemDatabase/main/scripts/install.sh
bash ./install.sh
rystemdb --version
```

Use `RYSTEMDB_VERSION=v0.1.2` to select a specific published version, `RYSTEMDB_INSTALL_DIR` to change the destination. Installer checks SHA-256 and puts the binary in `~/.local/bin` by default; add that folder to PATH.

## Windows (PowerShell)

```powershell
Invoke-WebRequest https://raw.githubusercontent.com/KeyserDSoze/RystemDatabase/main/scripts/install.ps1 -OutFile install-rystemdb.ps1
powershell -NoProfile -File .\install-rystemdb.ps1
rystemdb --version
```

The user PATH is updated for subsequent terminals. For a pinned release pass `-Version v0.1.2`.

## Document CRUD CLI (from v0.1.2, after verified publication)

```sh
printf '{"name":"Ada","active":true}' | rystemdb document put --data ./data --collection users --id u42 --json -
rystemdb document get --data ./data --collection users --id u42
rystemdb document delete --data ./data --collection users --id u42
```

See [typed JSON wire contract](DOCUMENT_CLI.md). This CLI accesses local filesystem storage and currently has no SQL, foreign-key integrity or multi-document transactions.

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

Published images (after the version-tag workflow): `ghcr.io/keyserdsoze/rystemdatabase:v0.1.2`.

```sh
docker volume create rystem-data
printf '{"name":"Ada"}' | docker run --rm -i -v rystem-data:/data ghcr.io/keyserdsoze/rystemdatabase:v0.1.2 \
  document put --data /data --collection users --id u42 --json -
docker run --rm -v rystem-data:/data ghcr.io/keyserdsoze/rystemdatabase:v0.1.2 \
  document get --data /data --collection users --id u42
```

The container uses a non-root user and a persistent `/data` volume for document CLI operations. Its network server only exposes health endpoints. Not production-ready.
