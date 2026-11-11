# Linux image for building and testing the desktop client away from Windows:
# `cargo test`, the x86_64-pc-windows-gnu cross-build and headless screenshots.
# The base image is pinned by digest. desktop/README.md shows how it is used.
FROM docker.io/library/rust:1.99.1-bookworm@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c

# GTK/WebKit headers for wry and rfd, MinGW for the Windows target and windres,
# Xvfb with Mesa's software renderers and fonts that have Cyrillic.
RUN apt-get update \
 && apt-get install -y --no-install-recommends \
      libgtk-3-dev libwebkit2gtk-4.1-dev libsoup-3.0-dev \
      gcc-mingw-w64-x86-64 \
      xvfb libgl1-mesa-dri libegl1 mesa-vulkan-drivers libxkbcommon-x11-0 \
      fonts-dejavu-core fonts-noto-core \
 && rm -rf /var/lib/apt/lists/*

# The Windows target of the same toolchain, for the cross-build.
RUN rustup target add x86_64-pc-windows-gnu

# The sources are bind-mounted from a checkout owned by uid 1000. Build output
# and the crate cache stay in volumes, never in the repository.
RUN useradd --create-home --uid 1000 dev \
 && mkdir -p /target /usr/local/cargo/registry /usr/local/cargo/git \
 && chown dev:dev /target /usr/local/cargo/registry /usr/local/cargo/git
ENV CARGO_TARGET_DIR=/target
USER dev
WORKDIR /work/desktop
