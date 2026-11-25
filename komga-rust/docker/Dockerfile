# syntax=docker/dockerfile:1.7

FROM --platform=$TARGETPLATFORM alpine:3.23 AS runner
ARG TARGETOS
ARG TARGETARCH
RUN <<'EOF'
set -eu
apk add --no-cache tzdata
mkdir -p /app
install -m 755 /dev/null /app/docker-runtime.sh
cat > /app/docker-runtime.sh <<'SCRIPT'
#!/bin/sh
set -eu

command="${1:-run}"
if [ "$command" = "run" ]; then
  shift
  exec /app/komga-rust "$@"
fi

if [ "$command" = "healthcheck" ]; then
  url_path="${SERVER_SERVLET_CONTEXT_PATH:-}/actuator/health"
  for host in 127.0.0.1 '[::1]'; do
    if response="$(wget -qO- "http://${host}:${SERVER_PORT:-25600}${url_path}" 2>/dev/null)"; then
      printf '%s' "$response" | grep -q '"status":"UP"'
      exit
    fi
  done
  exit 1
fi

exit 64
SCRIPT
EOF
WORKDIR /app
COPY --chmod=755 docker-artifacts/${TARGETOS}-musl/${TARGETARCH}/komga-rust ./komga-rust
VOLUME /config
ENV KOMGA_CONFIGDIR="/config"
ENV LANG='C.UTF-8' LC_ALL='C.UTF-8'
ENTRYPOINT ["./docker-runtime.sh", "run"]
HEALTHCHECK --interval=30s --timeout=5s --start-period=30s --retries=3 CMD ["./docker-runtime.sh", "healthcheck"]
EXPOSE 25600
LABEL org.opencontainers.image.source="https://github.com/huihuimoe/komga-riir"
