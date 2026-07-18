#!/bin/sh
set -eu

daemon_dir=${REFACT_DAEMON_DIR:-/var/lib/refact/daemon}
bind=${REFACT_DAEMON_BIND:-0.0.0.0}
port=${REFACT_DAEMON_PORT:-8488}
token=${REFACT_DAEMON_TOKEN:-}

case "$bind" in
    ''|*[!0-9A-Fa-f.:]*)
        printf 'invalid REFACT_DAEMON_BIND: %s\n' "$bind" >&2
        exit 1
        ;;
esac

case "$bind" in
    127.0.0.1|::1) ;;
    *)
        if [ -z "$token" ]; then
            printf '%s\n' 'REFACT_DAEMON_TOKEN is required for non-loopback Docker binds' >&2
            exit 1
        fi
        ;;
esac

case "$port" in
    ''|*[!0-9]*)
        printf 'invalid REFACT_DAEMON_PORT: %s\n' "$port" >&2
        exit 1
        ;;
esac
if [ "$port" -eq 0 ] || [ "$port" -gt 65535 ]; then
    printf 'invalid REFACT_DAEMON_PORT: %s\n' "$port" >&2
    exit 1
fi

case "$token" in
    *[!A-Za-z0-9._~-]*)
        printf '%s\n' 'REFACT_DAEMON_TOKEN may contain only letters, digits, dot, underscore, tilde, and hyphen' >&2
        exit 1
        ;;
esac

mkdir -p "$daemon_dir"
if [ -n "$token" ]; then
    auth_enabled=true
else
    auth_enabled=false
fi

cat > "$daemon_dir/daemon.yaml" <<EOF
port: $port
bind: '$bind'
auth:
  enabled: $auth_enabled
  token: '$token'
mdns:
  enabled: false
EOF

exec "$@"
