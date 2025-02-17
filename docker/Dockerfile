FROM lukemathwalker/cargo-chef:latest-rust-1 AS chef
WORKDIR /app

LABEL org.opencontainers.image.source=https://github.com/paradigmxyz/reth
LABEL org.opencontainers.image.licenses="MIT OR Apache-2.0"

RUN apt-get update && apt-get -y upgrade && apt-get install -y libclang-dev pkg-config

FROM chef AS planner

COPY rome-sdk /app/rome-sdk
COPY rome-evm-private /app/rome-evm-private
COPY reth /app/reth

WORKDIR /app/reth
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS builder
COPY --from=planner /app/reth/recipe.json /app/reth/recipe.json
COPY --from=planner /app/rome-sdk /app/rome-sdk
COPY --from=planner /app/rome-evm-private /app/rome-evm-private
COPY --from=planner /app/reth /app/reth

ARG BUILD_PROFILE=release
ENV BUILD_PROFILE=$BUILD_PROFILE

ARG RUSTFLAGS=""
ENV RUSTFLAGS="$RUSTFLAGS"

ARG FEATURES=""
ENV FEATURES=$FEATURES

WORKDIR /app/reth
RUN cargo chef cook --profile $BUILD_PROFILE --features "$FEATURES" --recipe-path recipe.json

RUN cargo build --profile $BUILD_PROFILE --features "$FEATURES" --locked --bin reth

RUN cp /app/reth/target/$BUILD_PROFILE/reth /app/reth

FROM ubuntu AS runtime
WORKDIR /app

COPY --from=builder /app/reth /usr/local/bin

COPY reth/LICENSE-* ./

EXPOSE 30303 30303/udp 9001 8545 8546
ENTRYPOINT ["/usr/local/bin/reth"]
