# Docker

Local build: `docker compose -f docker/compose.yml up --build`.

Once a versioned release has published to GHCR, images will be available under `ghcr.io/keysersdoze/rystemdatabase:vX.Y.Z`.

The current container is a **foundation health server only**. It has no persistence and intentionally no mounted data volume. Do not deploy as a database.
