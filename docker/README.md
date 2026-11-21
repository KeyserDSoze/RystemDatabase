# Docker (experimental Rystem Database)

Build and start the **health-only network server** with a named persistent data volume:

```sh
docker compose -f docker/compose.yml up --build -d
curl http://127.0.0.1:8787/healthz
```

The document CLI is **not exposed on the network**. Use the container itself to write and retrieve native binary documents:

```sh
printf '{"name":"Ada"}' | docker compose -f docker/compose.yml exec -T rystemdb \
  rystemdb document put --data /data --collection users --id u42 --json -

docker compose -f docker/compose.yml exec -T rystemdb \
  rystemdb document get --data /data --collection users --id u42
```

The `rystem-data` named volume survives container restarts/re-creation. The image runs as an unprivileged user with access to the in-image `/data` directory. Separate CLI invocations acquire the OS advisory lock.

After the tagged release, the equivalent immutable image is `ghcr.io/keyserdsoze/rystemdatabase:v0.1.2`. Verify the tag exists before using it.

**Status:** experimental single-document binary storage only. No network CRUD, SQL, authentication, backup/restore, MVCC or multi-document transactions. Do not deploy as a production database.
