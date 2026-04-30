ARG RUST_TAG=1.95-slim
FROM rust:${RUST_TAG} as builder

RUN apt-get update && apt-get install -y musl-tools musl-dev
RUN rustup target add x86_64-unknown-linux-musl
WORKDIR /usr/src/tube
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl

FROM alpine:latest

COPY --from=builder /usr/src/tube/target/x86_64-unknown-linux-musl/release/tube /usr/local/bin/tube
RUN chmod +x /usr/local/bin/tube
CMD ["cp", "/usr/local/bin/tube", "/shared/tube"]
