# Build: the frontend (wasm, via Trunk) and the server binary that embeds it.
FROM rust:1-bookworm AS build
ARG TRUNK_VERSION=0.21.14
RUN apt-get update && apt-get install -y --no-install-recommends protobuf-compiler libssl-dev pkg-config \
 && rm -rf /var/lib/apt/lists/* \
 && rustup target add wasm32-unknown-unknown \
 && curl -fsSL https://github.com/trunk-rs/trunk/releases/download/v${TRUNK_VERSION}/trunk-$(uname -m)-unknown-linux-gnu.tar.gz \
    | tar -xz -C /usr/local/bin
WORKDIR /src
COPY . .
RUN make dist

# Run: one binary plus one SQLite file in /data.
FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends libssl3 \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 doris && mkdir /data && chown doris /data
COPY --from=build /src/target/dist/doris /usr/local/bin/doris
USER doris
VOLUME /data
ENV DORIS_LISTEN=0.0.0.0:3000 \
    DORIS_DATABASE=sqlite:///data/doris.db
EXPOSE 3000
ENTRYPOINT ["doris"]
