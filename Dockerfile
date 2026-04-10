FROM rust:1.93-alpine3.20 as base
RUN apk add --no-cache musl-dev
RUN cargo install cargo-chef --version ^0.1

FROM base AS planner
WORKDIR /src
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

FROM base AS build
WORKDIR /src
COPY --from=planner /src/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --frozen

FROM mirror.gcr.io/alpine:3.20

RUN apk add --no-cache bash jq curl kubectl
COPY --from=build /src/target/release/kanidm-provision /usr/local/bin/kanidm-provision

COPY ./docker/sidecar/lib.jq /lib.jq
COPY ./docker/sidecar/entrypoint.sh /entrypoint.sh

USER 65532:65532
ENTRYPOINT ["/entrypoint.sh"]
