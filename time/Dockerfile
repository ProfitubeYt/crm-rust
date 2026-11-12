# «Тайм-лист» для сервера: одна программа, одна база SQLite в томе /data.
# Собирается тем же закреплённым образом Rust, что и настольный клиент
# (desktop/tools/linux-dev.Dockerfile), с Cargo.lock как есть.
FROM docker.io/library/rust:1.99.1-bookworm@sha256:e6f2cff35d0f309acf8433b4f05878ed77092602c5065cc81801177da0bb3a9c AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build --release --locked -p timelist

FROM docker.io/library/debian:bookworm-slim@sha256:734a72b3d6142a71993b4bc580ca01d8ca687f77a2c422d96a961d3376b179bc
# curl нужен только проверке живости; корневые сертификаты — для api.telegram.org.
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --home-dir /data --shell /usr/sbin/nologin timelist \
 && mkdir -p /data \
 && chown timelist:timelist /data
COPY --from=build /src/target/release/timelist /usr/local/bin/timelist
USER timelist
WORKDIR /data
# Порт слушается внутри сети контейнеров и наружу не публикуется: в режиме общих
# учётных записей к API ходит только шлюз сервера учётных записей.
ENV TIMELIST_DB=/data/timelist.db \
    TIMELIST_LISTEN=0.0.0.0:8080
VOLUME /data
EXPOSE 8080
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD curl -fsS http://127.0.0.1:8080/api/meta >/dev/null || exit 1
ENTRYPOINT ["timelist"]
