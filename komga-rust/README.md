# komga-rust

A Rust server for [Komga](https://komga.org), the media server for comics, mangas, BDs and
magazines. One binary serves both web UIs, the REST API, OPDS 1 and 2, Kobo sync and KOReader
sync, and keeps the behaviour of Kotlin Komga. It opens the data directories of Kotlin Komga
1.11.1 or later and of komga-riir releases, so an existing installation upgrades in place.

- [Run from a release archive](#run-from-a-release-archive)
- [Run in a container](#run-in-a-container)
- [Configuration](#configuration)
- [Upgrade from Kotlin Komga or komga-riir](#upgrade-from-kotlin-komga-or-komga-riir)
- [Roll back](#roll-back)
- [Build from source](#build-from-source)

## Run from a release archive

Each release has one archive per platform, `komga-rust-<version>-<target>.tar.gz` (`.zip` on
Windows), holding the `komga-rust` binary and its third-party licenses. The binary has no runtime dependencies.

| Platform | Target |
| --- | --- |
| Linux x86_64 | `x86_64-unknown-linux-musl` |
| Linux arm64 | `aarch64-unknown-linux-musl` |
| Windows x86_64 | `x86_64-pc-windows-msvc` |
| macOS Apple silicon | `aarch64-apple-darwin` |

```sh
tar -xzf komga-rust-<version>-<target>.tar.gz
cd komga-rust-<version>-<target>
./komga-rust
```

Open `http://<host>:25600` and create the first user. Data is kept in `~/.komga` unless
`--config-dir` says otherwise.

## Run in a container

The image is `ghcr.io/huihuimoe/komga-riir`, tagged with the release version and `latest`, for
`linux/amd64` and `linux/arm64`. It keeps its data in the `/config` volume and listens on port
25600.

### Allow io_uring

On Linux, komga-rust reads and writes files through io_uring. Docker 25 and later block the
`io_uring_setup`, `io_uring_enter` and `io_uring_register` system calls in their default seccomp
profile. The server still runs under that profile, but its file I/O falls back to the slower
thread pool. Allow the three calls with a seccomp profile:

```sh
curl -fsSL https://raw.githubusercontent.com/moby/profiles/main/seccomp/default.json \
  | jq '.syscalls += [{"names": ["io_uring_setup", "io_uring_enter", "io_uring_register"], "action": "SCMP_ACT_ALLOW"}]' \
  > seccomp-io-uring.json
```

This is Docker's default profile plus the io_uring calls. If you do not need the confinement,
`seccomp=unconfined` also works. Other container runtimes need the same three calls allowed if
their default profile blocks them. A host with the sysctl `kernel.io_uring_disabled` set to 1 or
2 also falls back to the thread pool.

### docker run

```sh
docker run -d --name komga \
  --security-opt seccomp=./seccomp-io-uring.json \
  -p 25600:25600 \
  -v /path/to/config:/config \
  -v /path/to/books:/books:ro \
  -e TZ=Europe/Paris \
  --restart unless-stopped \
  ghcr.io/huihuimoe/komga-riir:latest
```

Drop `:ro` from the books mount if you use features that write to the library, such as deleting
books, importing or converting.

### Docker Compose

```yaml
services:
  komga:
    image: ghcr.io/huihuimoe/komga-riir:latest
    security_opt:
      - seccomp=./seccomp-io-uring.json
      # Or, without generating a profile, turn seccomp off for this container:
      # - seccomp=unconfined
    ports:
      - 25600:25600
    volumes:
      - /path/to/config:/config
      - /path/to/books:/books:ro
    environment:
      TZ: Europe/Paris
      # Sort order for names: und (default), a language such as sv or ja,
      # or zh-u-co-pinyin, zh-u-co-stroke, zh-u-co-zhuyin.
      # KOMGA_SORT_LOCALE: zh-u-co-pinyin
      # Per-library override, as <library id>=<locale>,...
      # KOMGA_SORT_LIBRARYLOCALES: 0ABCDEFGHIJKL=ja
      # Peers allowed to set Forwarded / X-Forwarded-* headers: none, private
      # (default: loopback and private networks), all, or a comma-separated
      # list of IPs and CIDRs.
      # KOMGA_SERVER_TRUSTEDPROXIES: 172.16.0.0/12,10.0.0.5
    restart: unless-stopped
```

The image has a health check on `/actuator/health`. Arguments after the image name are passed to
`komga-rust`, for example `docker exec komga ./komga-rust --list-users`.

To build the image yourself, place the linux-musl binaries at
`docker-artifacts/linux-musl/<amd64|arm64>/komga-rust` in the build context and build
[`docker/Dockerfile`](docker/Dockerfile).

## Configuration

Settings come from, in order: command-line options, environment variables,
`<config-dir>/application.yml` (or `.yaml`, `.properties`), and defaults. Keys are those of Kotlin
Komga, so an existing `application.yml` keeps working. Environment variable names are the key in
upper case, with `.` replaced by `_` and `-` dropped or replaced by `_`: `komga.config-dir` is
`KOMGA_CONFIGDIR` or `KOMGA_CONFIG_DIR`.

| Key | Option | Default |
| --- | --- | --- |
| `komga.config-dir` | `--config-dir` | `~/.komga` (`/config` in the image) |
| `server.port` | `--port` | `25600` |
| `server.address` | `--host` | `::`, or `0.0.0.0` without IPv6 |
| `server.servlet.context-path` | | empty; set when served under a sub-path |
| `komga.server.trusted-proxies` | | loopback and private networks; peers whose `Forwarded` / `X-Forwarded-*` headers are trusted |
| `logging.file.name` | | `<config-dir>/logs/komga.log` |
| `komga.database.file` | | `<config-dir>/database.sqlite` |
| `komga.tasks-db.file` | | `<config-dir>/tasks.sqlite` |
| `komga.lucene.data-directory` | | `<config-dir>/lucene` |
| `spring.security.oauth2.client.*` | | OAuth2 / OIDC login providers |

`./komga-rust --help` lists every setting with its default. `--dry-run` resolves the
configuration and exits.

The databases must be on a local filesystem. Startup fails when one is on NFS or CIFS. If the
check is wrong for your setup, set `komga.database.check-local-filesystem` or
`komga.tasks-db.check-local-filesystem` to `false`.

Admin commands run against the config directory and exit:

```sh
komga-rust --list-users
komga-rust --reset alice@example.org --newpassword new-secret
```

## Upgrade from Kotlin Komga or komga-riir

Supported sources:

- Kotlin Komga 1.11.1 or later (database migration `20240422132621` applied). Upgrade an older
  installation with Kotlin Komga first.
- komga-riir releases.

Databases with unknown or newer migrations are refused and left untouched.

1. Stop the running server. Do not run Kotlin Komga, komga-riir and komga-rust on the same config
   directory at the same time. komga-rust waits while another komga-rust holds the directory
   lock, but the older servers do not take that lock.
2. Make sure the disk has free space of at least the size of `database.sqlite` plus
   `tasks.sqlite`.
3. Start komga-rust with the same config directory. In a container, mount the old config
   directory at `/config`. Before it changes a database, komga-rust writes a consistent copy of
   both databases to `<config-dir>/backups/pre-upgrade-<UTC time>/`. Then it rebuilds the search
   index in `<config-dir>/lucene/tantivy`.

Backup sets are kept until you delete them.

## Roll back

The previous server cannot open an upgraded database. Restore the backup set instead. Changes
made since the upgrade are lost.

1. Stop komga-rust.
2. In the config directory, delete `database.sqlite-wal`, `database.sqlite-shm`,
   `tasks.sqlite-wal` and `tasks.sqlite-shm` if present.
3. Copy `database.sqlite` and `tasks.sqlite` from `backups/pre-upgrade-<UTC time>/` into the config
   directory.
4. Start the previous server.

## Build from source

Requires the stable Rust toolchain. The workspace builds with `--cfg tokio_unstable`, which
[`.cargo/config.toml`](.cargo/config.toml) sets.

```sh
cargo build --release -p komga-rust
cargo nextest run          # tests
```

Before committing changes, install the code-budget pre-commit hook. It checks the staged Rust
files under `komga-rust/` against the file and function size budgets and blocks the commit when a
hard limit is exceeded. Run this once from the repository root:

```sh
ln -s "$PWD/komga-rust/tools/code-budget/pre-commit" "$(git rev-parse --git-path hooks)/pre-commit"
```

If you already have a pre-commit hook, add the line `komga-rust/tools/code-budget/pre-commit || exit $?`
to it instead. Approved exceptions go in
[`tools/code-budget/exceptions.toml`](tools/code-budget/exceptions.toml).

[`tools/release/package.sh`](tools/release/package.sh) `<target> <version>` builds one release
archive into `dist/`. It needs `cargo-about` on `PATH`.
