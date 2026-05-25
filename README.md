# navio-signer

Signing daemon for [navio-core](https://github.com/nav-io/navio-core) guix
release artifacts.

## What it does

`navio-signer` polls navio-core for new completed guix workflow runs,
downloads the per-HOST artifacts, codesigns the binaries on a maintainer-
controlled mac, and publishes the signed bundles to GitHub Releases on
navio-core.

Signing splits by platform:

| HOST                    | Signing                                               |
| ----------------------- | ----------------------------------------------------- |
| `*-linux-gnu`           | GPG detach-sign the tarball                           |
| `*-linux-gnueabihf`     | GPG detach-sign the tarball                           |
| `x86_64-w64-mingw32`    | `osslsigncode` (Authenticode) each `.exe` / `.dll`    |
| `*-apple-darwin`        | `codesign` + `xcrun notarytool` (no stapling — bare Mach-O) |

A top-level `SHA256SUMS` covering all signed assets is generated and
GPG-detach-signed (`SHA256SUMS.asc`) per release.

Releases:

| Trigger                | Tag                  | Pre-release |
| ---------------------- | -------------------- | ----------- |
| `refs/heads/master`    | `nightly` (rolling)  | yes         |
| `refs/tags/vX.Y.Z`     | `vX.Y.Z`             | no          |
| `refs/tags/vX.Y.Z-rc*` | `vX.Y.Z-rc*`         | yes         |

## Status

Early scaffold. Phase 1 (this milestone) lays down the CLI skeleton, config
loader, sqlite state machine, and launchd plist. The actual signing and
publishing logic lands in subsequent phases — track via
[issues](https://github.com/mxaddict/navio-signer/issues).

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

Fill in:

- `github.token` — a GitHub PAT with `actions:read` + `contents:write` on the
  source repo
- `signing.linux.gpg_key_id` — GPG key fingerprint
- `signing.windows.pkcs12_path` + `pkcs12_password` — Authenticode cert
- `signing.macos.identity` + `keychain_profile` — Developer ID + notarytool
  keychain profile

See [`config.toml.example`](config.toml.example) for the full schema.

## Run

### One-shot

```sh
navio-signer poll
navio-signer fetch <RUN_ID>
navio-signer verify <RUN_ID>
navio-signer sign <RUN_ID>
navio-signer publish <RUN_ID>
navio-signer status
```

### Daemon (manual)

```sh
navio-signerd
```

Logs go to stdout/stderr at `info` level (release) or `debug` (debug builds).
Override via `--log-level` or `RUST_LOG`.

### Daemon (launchd)

```sh
cargo install --path . --root /usr/local
./contrib/install.sh
```

Loads `~/Library/LaunchAgents/sh.navio.signer.plist`. Logs land in
`~/Library/Logs/navio-signer/{stdout,stderr}.log`.

To stop:

```sh
launchctl unload ~/Library/LaunchAgents/sh.navio.signer.plist
```

## Operator setup

Required tools on the signer host (macOS, hardware-assisted):

- Rust toolchain (`rustup`)
- `osslsigncode` (`brew install osslsigncode`)
- Xcode Command Line Tools (`codesign`, `xcrun notarytool`)
- GnuPG (`brew install gnupg`)

One-time credential bootstrap:

```sh
# Apple notarization credentials (stored in keychain under the profile name)
xcrun notarytool store-credentials "navio-notary" \
    --apple-id you@example.com \
    --team-id TEAMID \
    --password "<app-specific-password>"

# GPG key — generate or import; note the long fingerprint for config
gpg --list-secret-keys --keyid-format=long

# Windows cert — obtain a code-signing PKCS#12 from a CA, store at the path
# referenced in config (file mode 0600, disk-encrypted volume recommended)
```

## License

MIT. See [LICENSE](LICENSE).
