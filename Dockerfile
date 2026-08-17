FROM rust:1.83 AS builder
WORKDIR /app
COPY . .
RUN cargo build --release

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
    libssl3 ca-certificates && rm -rf /var/lib/apt/lists/*
ENV TERM=xterm-256color
COPY --from=builder /app/target/release/termos /usr/local/bin/termos
ENTRYPOINT ["termos"]
