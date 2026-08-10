ARG BASE_IMAGE=debian:bookworm

FROM ${BASE_IMAGE}

ARG LINUX_TARGET_TRIPLE=x86_64-unknown-linux-gnu
ARG RUST_TOOLCHAIN=1.95.0
# The native window's terminal is libghostty-vt, built from Ghostty's Zig source.
ARG ZIG_VERSION=0.15.2

ENV DEBIAN_FRONTEND=noninteractive
ENV RUSTUP_HOME=/opt/rust/rustup
ENV CARGO_HOME=/opt/rust/cargo
ENV PATH=/opt/zig:/opt/rust/cargo/bin:$PATH
ENV CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=x86_64-linux-gnu-gcc
ENV CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER=aarch64-linux-gnu-gcc

RUN apt-get update \
    && apt-get install -y --no-install-recommends \
        build-essential \
        ca-certificates \
        curl \
        gcc-aarch64-linux-gnu \
        gcc-x86-64-linux-gnu \
        libc6-dev-arm64-cross \
        libc6-dev-amd64-cross \
        git \
        nodejs \
        npm \
        pkg-config \
        xz-utils \
    && rm -rf /var/lib/apt/lists/*

# Zig builds libghostty-vt, and it cross-compiles it for whichever target Cargo asks for,
# so only the builder's own architecture matters here.
RUN set -eux; \
    case "$(uname -m)" in \
        x86_64) zig_arch=x86_64 ;; \
        aarch64 | arm64) zig_arch=aarch64 ;; \
        *) echo "unsupported builder architecture $(uname -m)" >&2; exit 1 ;; \
    esac; \
    curl --proto "=https" --tlsv1.2 -sSfL \
        "https://ziglang.org/download/${ZIG_VERSION}/zig-${zig_arch}-linux-${ZIG_VERSION}.tar.xz" \
        -o /tmp/zig.tar.xz; \
    mkdir -p /opt/zig; \
    tar -xJf /tmp/zig.tar.xz -C /opt/zig --strip-components=1; \
    rm /tmp/zig.tar.xz; \
    chmod -R a+rX /opt/zig; \
    zig version

RUN curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs \
        | sh -s -- -y --profile minimal --default-toolchain "${RUST_TOOLCHAIN}" --target "${LINUX_TARGET_TRIPLE}" \
    && chmod -R a+rX /opt/rust
