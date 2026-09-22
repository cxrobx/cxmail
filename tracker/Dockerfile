FROM rust:latest AS builder

# Install Tauri build dependencies (needed because tracker links against cxmail_lib)
RUN apt-get update && apt-get install -y \
    libgtk-3-dev \
    libwebkit2gtk-4.1-dev \
    libappindicator3-dev \
    librsvg2-dev \
    patchelf \
    pkg-config \
    libssl-dev \
    libclang-dev \
    clang \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /build
COPY src-tauri/ ./

# Build only the tracker binary
RUN cargo build --release --bin cxmail-tracker

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/cxmail-tracker /usr/local/bin/cxmail-tracker

RUN mkdir -p /data
VOLUME /data

ENV RUST_LOG=info

EXPOSE 8090

ENTRYPOINT ["cxmail-tracker"]
CMD ["serve", "--port", "8090", "--db-path", "/data/tracker.db"]
