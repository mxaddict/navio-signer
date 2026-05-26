# Configuration

`config.toml` lives in one of:

1. `./config.toml` (cwd at daemon launch — highest precedence)
2. `$XDG_CONFIG_HOME/navio-signer/config.toml`
   - Linux: `~/.config/navio-signer/config.toml`
   - macOS: `~/Library/Application Support/navio-signer/config.toml`
3. Explicit path via `--config <PATH>` or `NAVIO_SIGNER_CONFIG=<PATH>`

The daemon exits with a clear error if no config file is found.

All paths in this file may be absolute or shell-style (`~`, `$HOME`). Plaintext
secrets (PKCS#12 password) live on local disk — keep file mode **0600** and
prefer a disk-encrypted volume.

A working template ships as [`config.toml.example`](../config.toml.example) at
the repo root.

---

## `[github]`

GitHub API access + repo coordinates.

| Key           | Type     | Default                                 | Description                                                      |
| ------------- | -------- | --------------------------------------- | ---------------------------------------------------------------- |
| `token`       | string   | —                                       | **Required.** Personal access token. See scopes below.           |
| `source_repo` | string   | —                                       | **Required.** `owner/name` to poll, e.g. `nav-io/navio-core`.    |
| `workflow`    | string   | —                                       | **Required.** Workflow filename to filter runs, e.g. `guix.yml`. |
| `refs`        | string[] | `["refs/heads/master", "refs/tags/v*"]` | Ref patterns to sign (see below).                                |

### `token` scopes

- `actions:read` on `source_repo` — list workflow runs, download artifacts
- `contents:write` on `source_repo` — create releases, upload assets

Fine-grained, repo-scoped tokens are recommended.

### `refs` pattern syntax

- `refs/heads/<branch>` — exact branch match
- `refs/tags/<name>` — exact tag match
- `refs/tags/<prefix>*` — tag prefix glob (e.g. `refs/tags/v*` matches `v1.0.0`,
  `v0.1.0-rc1`)

Anything not matching one of these prefixes is rejected at startup.

---

## `[signing.linux]`

GPG detach-signing for linux tarballs and the top-level `SHA256SUMS`.

| Key          | Type   | Default | Description                                              |
| ------------ | ------ | ------- | -------------------------------------------------------- |
| `gpg_key_id` | string | —       | **Required.** GPG key fingerprint (long-form preferred). |

`gpg` must be on `PATH`. Signing uses
`--batch --yes --detach-sign --armor --local-user <key>` — see
[SETUP.md](SETUP.md) for keygen + cache config.

---

## `[signing.windows]`

Authenticode signing for mingw `.exe` and `.dll` files via `osslsigncode`.

| Key               | Type   | Default                        | Description                                       |
| ----------------- | ------ | ------------------------------ | ------------------------------------------------- |
| `pkcs12_path`     | path   | —                              | **Required.** Absolute path to the `.p12` bundle. |
| `pkcs12_password` | string | —                              | **Required.** Plaintext password for the PKCS#12. |
| `timestamp_url`   | string | `http://timestamp.sectigo.com` | RFC3161 TSA used by `osslsigncode -t`.            |

`osslsigncode` must be on `PATH` (`brew install osslsigncode`).

The signer iterates every `.exe` / `.dll` inside the mingw `.zip`, signs each in
a tempdir, and repacks the archive deterministically (preserves entry order,
compression, and original mtimes).

---

## `[signing.macos]`

Codesigning + notarization for Mach-O binaries inside darwin tarballs.

| Key                | Type   | Default | Description                                                                                  |
| ------------------ | ------ | ------- | -------------------------------------------------------------------------------------------- |
| `identity`         | string | —       | **Required.** codesign identity string. Find via `security find-identity -p codesigning -v`. |
| `keychain_profile` | string | —       | **Required.** notarytool keychain profile name (see [SETUP.md](SETUP.md)).                   |

No stapling — bare Mach-O binaries can't be stapled (no embedded container for
the ticket). Gatekeeper resolves the notarization ticket online.

No entitlements — depends/ static-links everything in navio, nothing requires
sandbox/hardening overrides.

---

## `[daemon]`

| Key                  | Type | Default | Description                                                        |
| -------------------- | ---- | ------- | ------------------------------------------------------------------ |
| `poll_interval_secs` | u64  | `60`    | Seconds between workflow-run polls. The whole section is optional. |

---

## `[paths]`

Optional. Both keys default to the XDG-derived locations.

| Key        | Type | Default                       | Description                      |
| ---------- | ---- | ----------------------------- | -------------------------------- |
| `data_dir` | path | `$XDG_DATA_HOME/navio-signer` | State DB + per-run workdir root. |

Override when running under a service account that needs a fixed path (e.g.
`/var/lib/navio-signer` for a non-launchd setup).

---

## Environment variables

| Variable              | Effect                                                                |
| --------------------- | --------------------------------------------------------------------- |
| `NAVIO_SIGNER_CONFIG` | Equivalent to `--config <PATH>`.                                      |
| `NAVIO_SIGNER_LOG`    | Equivalent to `--log-level <LEVEL>` (overrides `RUST_LOG`).           |
| `RUST_LOG`            | Standard tracing-subscriber filter, used when `--log-level` is unset. |

---

## Filesystem layout

State and per-run work directories under `data_dir` (default
`$XDG_DATA_HOME/navio-signer`):

```
data_dir/
├── builds.db                     # sqlite state DB
├── navio-signer.lock             # PID lockfile (daemon only)
└── work/
    └── <run_id>/
        ├── x86_64-linux-gnu/     # extracted artifact dirs (per HOST)
        │   ├── navio-*.tar.gz
        │   └── navio-*.tar.gz.asc
        ├── x86_64-w64-mingw32/
        │   └── navio-*.zip
        ├── x86_64-apple-darwin/
        │   └── navio-*.tar.gz
        ├── SHA256SUMS
        └── SHA256SUMS.asc
```

The workdir is removed on successful publish; failed runs keep it around for
inspection.
