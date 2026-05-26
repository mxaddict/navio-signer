# navio-signer

Signing daemon for [navio-core](https://github.com/nav-io/navio-core) guix
release artifacts.

## What it does

`navio-signer` polls navio-core for new successful guix workflow runs, downloads
the per-HOST artifacts, codesigns the binaries on a maintainer- controlled mac,
and publishes the signed bundles to GitHub Releases on navio-core.

Signing splits by platform:

| HOST                 | Signing                                                     |
| -------------------- | ----------------------------------------------------------- |
| `*-linux-gnu*`       | GPG detach-sign the tarball                                 |
| `x86_64-w64-mingw32` | `osslsigncode` (Authenticode) each `.exe` / `.dll`          |
| `*-apple-darwin`     | `codesign` + `xcrun notarytool` (no stapling — bare Mach-O) |

A top-level `SHA256SUMS` covering all signed assets is generated and
GPG-detach-signed (`SHA256SUMS.asc`) per release.

Releases:

| Trigger                                 | Tag                 | Pre-release |
| --------------------------------------- | ------------------- | ----------- |
| `refs/heads/master`                     | `nightly` (rolling) | yes         |
| `refs/tags/vX.Y.Z`                      | `vX.Y.Z`            | no          |
| `refs/tags/vX.Y.Z-{rc,beta,alpha,pre}*` | same as tag         | yes         |

Existing releases are reused; assets are clobbered by name on re-upload.
Releases are never deleted.

## Build

```sh
cargo build --release
```

Produces two binaries in `target/release/`:

- `navio-signer` — subcommand-driven CLI
- `navio-signerd` — alias that invokes `navio-signer daemon`

## Configure

Copy `config.toml.example` to one of:

- `./config.toml` (cwd of the daemon)
- `$XDG_CONFIG_HOME/navio-signer/config.toml`
  (`~/Library/Application Support/navio-signer/config.toml` on macOS)

Then fill in the required keys. Full schema lives in
[`docs/CONFIG.md`](docs/CONFIG.md).

Signing-key bootstrap (Apple Developer ID, GPG, Windows .p12) lives in
[`docs/SETUP.md`](docs/SETUP.md).

## Run

### One-shot subcommands

```sh
navio-signer poll              # discover new runs, record in DB
navio-signer fetch   <RUN_ID>  # download artifacts
navio-signer verify  <RUN_ID>  # SHA256 against manifest
navio-signer sign    <RUN_ID>  # platform-specific signing
navio-signer publish <RUN_ID>  # upload to GH release
navio-signer status            # dump DB state
```

Each subcommand is a state gate — re-running a completed step is a no-op, and
failures park the build in `Failed` state until reset.

### Daemon (foreground)

```sh
navio-signerd
```

Equivalent to `navio-signer daemon`. Polls every 60s (configurable). Logs go to
stdout/stderr at `info` (release) or `debug` (debug builds). Override via
`--log-level` or `RUST_LOG`.

### Daemon (launchd)

```sh
cargo install --path . --root /usr/local
./contrib/install.sh
```

Setup, log paths, stop/restart in [`docs/LAUNCHD.md`](docs/LAUNCHD.md).

## Operator setup

Required tools on the signer host (macOS):

- Rust toolchain (`rustup`)
- `osslsigncode` (`brew install osslsigncode`)
- Xcode Command Line Tools (`codesign`, `xcrun notarytool`)
- GnuPG (`brew install gnupg`)

Step-by-step key bootstrap in [`docs/SETUP.md`](docs/SETUP.md).

## Docs

- [`docs/CONFIG.md`](docs/CONFIG.md) — config.toml schema
- [`docs/SETUP.md`](docs/SETUP.md) — signing-key bootstrap
- [`docs/LAUNCHD.md`](docs/LAUNCHD.md) — launchd agent
- [`docs/TROUBLESHOOTING.md`](docs/TROUBLESHOOTING.md) — common failures

## License

MIT. See [LICENSE](LICENSE).
