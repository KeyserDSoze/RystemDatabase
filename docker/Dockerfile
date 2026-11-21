FROM rust:1.85.1-bookworm AS builder
WORKDIR /build
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY src ./src
RUN cargo build --release --locked -p rystemdb-cli

FROM debian:bookworm-slim
RUN groupadd --system --gid 10001 rystem && useradd --system --uid 10001 --gid 10001 --no-create-home rystem
COPY --from=builder /build/target/release/rystemdb /usr/local/bin/rystemdb
USER 10001:10001
EXPOSE 8787
ENTRYPOINT ["rystemdb"]
CMD ["serve", "--listen", "0.0.0.0:8787"]
