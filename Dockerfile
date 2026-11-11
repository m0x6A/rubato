# syntax=docker/dockerfile:1
# rubato-server with the web view. Build from the repository root:
#   docker build --pull --platform linux/amd64 -t local/rubato:sha-$(git rev-parse --short HEAD) .

FROM node:22-bookworm-slim AS web
WORKDIR /src
RUN corepack enable
# Every workspace package's manifest, so the lockfile resolves; only web is installed and built.
COPY package.json pnpm-lock.yaml pnpm-workspace.yaml tsconfig.base.json ./
COPY web/package.json web/
COPY actions/plan-check/package.json actions/plan-check/
COPY skills/rubato/package.json skills/rubato/
COPY e2e/package.json e2e/
RUN --mount=type=cache,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile --filter ./web...
COPY web web
RUN pnpm --dir web build

FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates crates
# The launch prompt is embedded in the server at compile time.
COPY skills/rubato/prompt.md skills/rubato/prompt.md
# The target dir is a cache mount, so the binary is copied out of it in the same step.
RUN --mount=type=cache,target=/usr/local/cargo/registry \
    --mount=type=cache,target=/src/target \
    cargo build --release --locked -p rubato-server \
    && cp target/release/rubato-server /rubato-server

FROM debian:bookworm-slim
# ca-certificates for the GitHub API (rustls), wget for the container health check.
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates wget \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 1000 --user-group --no-create-home rubato \
    && mkdir /data && chown rubato:rubato /data
COPY --from=build /rubato-server /usr/local/bin/rubato-server
COPY --from=web /src/web/dist /app/web/dist
# Listen on every interface inside the container; the SQLite database lives on the /data volume.
ENV RUBATO_BIND=0.0.0.0:8080 \
    RUBATO_WEB_DIST=/app/web/dist \
    RUBATO_DATABASE_URL=sqlite:///data/rubato.db?mode=rwc
USER rubato
WORKDIR /data
VOLUME /data
EXPOSE 8080
CMD ["rubato-server"]
