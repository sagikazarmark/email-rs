FROM --platform=$BUILDPLATFORM tonistiigi/xx:1.10.0@sha256:40fe21f44bb8cddf1eb1226db190d0a984118f62ee02a91b77d99448d28264a6 AS xx

FROM --platform=$BUILDPLATFORM rust:1.99.0-slim@sha256:2752b332db73fdbb7dc576f06c82ed1f312005784ef913d7e04a28f5f55dc581 AS builder

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

FROM debian:13.7-slim@sha256:a29215f6a35e51e22adffa17f89e9d2ef06214e64a2bad10d765c46aea49f11f

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /usr/local/bin/restate-email /usr/local/bin/

ENV RUST_LOG=info

EXPOSE 9080

CMD ["restate-email"]
