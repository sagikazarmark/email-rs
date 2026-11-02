FROM --platform=$BUILDPLATFORM tonistiigi/xx:1.10.0@sha256:40fe21f44bb8cddf1eb1226db190d0a984118f62ee02a91b77d99448d28264a6 AS xx

FROM --platform=$BUILDPLATFORM rust:1.99.1-slim@sha256:b8e062b1c3a80183df83d9ba187cadd2b5ab84bc78d453dbfeb2fccd5f982247 AS builder

COPY --from=xx / /

RUN apt-get update && apt-get install -y clang lld

WORKDIR /usr/src/app

COPY . ./

RUN cargo fetch --locked

ARG TARGETPLATFORM

RUN xx-apt-get update && \
    xx-apt-get install -y \
    gcc \
    g++ \
    libc6-dev \
    pkg-config

RUN xx-cargo build --locked --release --bin restate-email
RUN xx-verify ./target/$(xx-cargo --print-target-triple)/release/restate-email
RUN cp ./target/$(xx-cargo --print-target-triple)/release/restate-email /usr/local/bin/restate-email

FROM debian:13.7-slim@sha256:f6b08b09a05725f9431502d3b5714f5c4576e82c1a84fa1debe0c24a7e0c3ec4

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/restate-email /usr/local/bin/

ENV RUST_LOG=info

EXPOSE 9080

CMD ["restate-email"]
