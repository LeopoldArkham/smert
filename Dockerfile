# syntax=docker/dockerfile:1

FROM rust:1-slim-bookworm AS build
WORKDIR /src
COPY . .
# Cache mounts keep downloaded crates and compiled dependencies between builds
# on the same host, so a redeploy only recompiles smert itself.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked && cp target/release/smert /usr/local/bin/smert

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 --home-dir /data smert \
 && install -d -o smert -g smert /data
COPY --from=build /usr/local/bin/smert /usr/local/bin/smert

# Mount a persistent volume here: it holds the database and images.
ENV SMERT_DATA_DIR=/data \
    SMERT_BIND=0.0.0.0:1212
VOLUME /data
EXPOSE 1212
USER smert
CMD ["smert"]
