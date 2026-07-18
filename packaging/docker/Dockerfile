FROM debian:bookworm-slim

ARG REFACT_VERSION=__REFACT_VERSION__
ARG REFACT_RELEASE_BASE_URL=https://github.com/JegernOUTT/refact/releases/download
ARG TARGETARCH

RUN apt-get update \
    && apt-get install --yes --no-install-recommends ca-certificates curl tar \
    && rm -rf /var/lib/apt/lists/* \
    && case "$TARGETARCH" in \
        amd64) target=x86_64-unknown-linux-gnu ;; \
        arm64) target=aarch64-unknown-linux-gnu ;; \
        *) echo "unsupported Docker architecture: $TARGETARCH" >&2; exit 1 ;; \
    esac \
    && archive="refact-${REFACT_VERSION}-${target}.tar.gz" \
    && url="${REFACT_RELEASE_BASE_URL%/}/engine/v${REFACT_VERSION}/${archive}" \
    && curl --fail --location --retry 3 --proto '=https' --tlsv1.2 --output "/tmp/${archive}" "$url" \
    && curl --fail --location --retry 3 --proto '=https' --tlsv1.2 --output "/tmp/${archive}.sha256" "${url}.sha256" \
    && cd /tmp \
    && sha256sum --check "${archive}.sha256" \
    && tar --extract --gzip --file "$archive" --directory /usr/local/bin refact \
    && chmod 0755 /usr/local/bin/refact \
    && rm -f "/tmp/${archive}" "/tmp/${archive}.sha256"

COPY entrypoint.sh /usr/local/bin/refact-container-entrypoint

RUN chmod 0755 /usr/local/bin/refact-container-entrypoint \
    && mkdir -p /projects /var/lib/refact/daemon \
    && useradd --create-home --home-dir /var/lib/refact --uid 10001 refact \
    && chown -R refact:refact /projects /var/lib/refact

ENV HOME=/var/lib/refact
ENV REFACT_DAEMON_DIR=/var/lib/refact/daemon
ENV REFACT_DAEMON_BIND=0.0.0.0
ENV REFACT_DAEMON_PORT=8488

USER refact
WORKDIR /projects
VOLUME ["/projects", "/var/lib/refact"]
EXPOSE 8488

ENTRYPOINT ["refact-container-entrypoint"]
CMD ["refact", "daemon", "--foreground"]
