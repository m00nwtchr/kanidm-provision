FROM rust:1.93-alpine3.22 as base
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

FROM gcr.io/distroless/static:nonroot
COPY --from=build /src/target/release/kanidm-provision /kanidm-provision
COPY --from=build /src/target/release/kanidm-provision-sidecar /kanidm-provision-sidecar

ENTRYPOINT ["/kanidm-provision"]
