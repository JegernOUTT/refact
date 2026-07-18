# Refact container

The image downloads and checksum-verifies a pinned Linux engine release at build
time, then runs `refact daemon --foreground` as an unprivileged user. It supports
Docker `amd64` and `arm64` builds.

From this directory, stamp the release version and build:

```sh
docker build --build-arg REFACT_VERSION=8.3.0 --tag refact:8.3.0 .
```

To build from a release mirror, also pass
`--build-arg REFACT_RELEASE_BASE_URL=https://mirror.example/releases/download`.

The daemon refuses a non-loopback bind unless `REFACT_DAEMON_TOKEN` is set. The
entrypoint writes the bind, port, and token to the daemon configuration before
startup. Run the Compose example with a long random token and the host project
directory to mount:

```sh
export REFACT_DAEMON_TOKEN="$(openssl rand -hex 32)"
export REFACT_PROJECT_PATH="$PWD"
docker compose up --build
```

Open `http://127.0.0.1:8488` and authenticate with the token. The project is
mounted at `/projects/workspace`; daemon state persists in the `refact-state`
volume. Keep port 8488 behind a firewall or trusted reverse proxy when exposing
it beyond the local machine.
