# Troubleshooting

Symptoms → likely cause → fix. Listed roughly in order of "things that will bite
you first".

## Recovery model

The state machine is `Discovered → Fetched → Verified → Signed → Published`. Any
step can transition to `Failed`, recording an error message. Failed builds stay
parked until reset.

```sh
# Inspect state for a run
navio-signer status

# Reset a Failed run to retry from scratch
sqlite3 "$XDG_DATA_HOME/navio-signer/builds.db" \
    "UPDATE builds SET state='discovered', error=NULL WHERE run_id=<ID>;"
rm -rf "$XDG_DATA_HOME/navio-signer/work/<ID>"
```

Re-running a non-Failed step is idempotent — `fetch` on a Fetched run is a
no-op, etc.

---

## Poll / fetch failures

### "404 Not Found" listing runs

- Token is missing `actions:read` for `source_repo`, or `source_repo` is wrong.
  Verify with `gh api repos/<owner>/<repo>/actions/runs`.

### Rate limited (`403 API rate limit exceeded`)

- `octocrab` shares the standard 5000/hr authenticated quota. With a 60s poll
  interval and ~3 API calls per cycle, you're at ~180/hr — well under the cap.
  If you see this, another process is sharing the token.
- Workaround: increase `daemon.poll_interval_secs` or rotate to a token on a
  different identity.

### Artifact download stalls or returns empty zip

- GHA artifacts older than 90 days expire. `fetch` will see them in the listing
  but `download_artifact` returns 410. Symptom: `Failed` with message mentioning
  `410`. Skip the run — there's nothing to sign.

---

## Verify failures

### "SHA256 mismatch"

The downloaded artifact doesn't match the manifest. Causes:

- Truncated download (network blip). Reset the run and re-fetch.
- guix produced a non-deterministic output (compiler version skew). File a
  navio-core issue and reset.

---

## Sign failures: GPG (linux)

### "gpg: signing failed: No secret key"

- `signing.linux.gpg_key_id` doesn't match an installed key. Verify with
  `gpg --list-secret-keys --keyid-format=long`.

### "gpg: signing failed: Inappropriate ioctl for device"

- `gpg-agent` tried to prompt for a passphrase on a TTY that doesn't exist
  (you're running under launchd). Either:
  - Use a passphraseless key, or
  - Configure `pinentry-mac` + warm the agent cache interactively (see
    [SETUP.md §2.4](SETUP.md)).

### "gpg: signing failed: Operation cancelled"

- Same root cause, manifested as a pinentry timeout. Same fixes.

---

## Sign failures: Authenticode (mingw)

### "osslsigncode: not found"

- `brew install osslsigncode`. Make sure `PATH` in
  `~/Library/LaunchAgents/sh.navio.signer.plist` includes `/opt/homebrew/bin`
  (already in the shipped template).

### "Failed to decrypt PKCS12 file"

- `pkcs12_password` in config doesn't match the `.p12`. Test with the command in
  [SETUP.md §3.4](SETUP.md).

### "Failed to add timestamp"

- The configured TSA is unreachable. Defaults to `http://timestamp.sectigo.com`;
  if it's down try `http://timestamp.digicert.com` or
  `http://timestamp.entrust.net`.

### Verify fails despite sign succeeding

- The cert may be expired or revoked. Check expiry:

  ```sh
  openssl pkcs12 -in ~/keys/navio-codesign.p12 -nokeys -info | \
      openssl x509 -noout -dates
  ```

---

## Sign failures: codesign + notarization (darwin)

### "errSecInternalComponent" from codesign

- Login keychain is locked. Unlock once:

  ```sh
  security unlock-keychain ~/Library/Keychains/login.keychain-db
  ```

  Use a LaunchAgent (not LaunchDaemon) so the keychain unlocks at login.

### "Invalid signature" after codesign

- Wrong identity. Verify the exact identity string with
  `security find-identity -p codesigning -v` and paste it verbatim into
  `signing.macos.identity`.

### Notarization "Invalid" / rejected

`xcrun notarytool submit ... --wait` returns status `Invalid` with a ticket
UUID. Pull the detailed log:

```sh
xcrun notarytool log <UUID> --keychain-profile navio-notary
```

Common rejections + fixes:

| Notary error                                                | Cause                                                             | Fix                                                                         |
| ----------------------------------------------------------- | ----------------------------------------------------------------- | --------------------------------------------------------------------------- |
| `The signature does not include a secure timestamp`         | codesign omitted `--timestamp`.                                   | Already in the signer — if you see this, you've patched it locally; revert. |
| `The executable does not have the hardened runtime enabled` | Missing `--options runtime`.                                      | Same — already in the signer.                                               |
| `The binary is not signed`                                  | A `.dylib` inside the tarball got missed.                         | File a bug; the sign step should pick up everything matching Mach-O magic.  |
| `Team ID mismatch`                                          | `keychain_profile` was set up for a different team than the cert. | Re-run `xcrun notarytool store-credentials` with the right `--team-id`.     |

### Notarization timeout

`--wait` polls Apple's service. If it exits with a clear "still in progress"
message after several minutes, you can resume:

```sh
xcrun notarytool info <UUID> --keychain-profile navio-notary --wait
```

The signer surfaces the UUID in the log line; copy it out and resume manually,
then reset the run state to `signed`.

---

## Publish failures

### "release with tag already exists" + retry doesn't help

- Shouldn't happen — the publisher uses get-or-create. If it does, the tag
  exists on the **source repo** but not via the API (cache or race). Wait 30s
  and re-run.

### "asset already exists" on upload

- Should also be auto-clobbered. If it persists, the previous asset is stuck in
  a half-uploaded state. Delete manually:

  ```sh
  gh api repos/nav-io/navio-core/releases/<RELEASE_ID>/assets \
      --jq '.[] | select(.name=="<ASSET>") | .id' | \
      xargs -I{} gh api -X DELETE \
          repos/nav-io/navio-core/releases/assets/{}
  ```

### CHANGELOG section missing

- The publisher fetches `CHANGELOG.md` at the source ref and looks for
  `## [Unreleased]` (nightly) or `## [X.Y.Z]` (tag). If missing, the release
  body is empty and a warning is logged. Edit the release manually in the GH UI;
  the next publish on the same tag will not clobber the body (only assets).

---

## Daemon won't stay up under launchd

```sh
tail -200 ~/Library/Logs/navio-signer/stderr.log
```

If you see a stack trace, fix the underlying issue. If you see nothing, the
binary may be exiting immediately at startup — try foreground:

```sh
launchctl unload ~/Library/LaunchAgents/sh.navio.signer.plist
NAVIO_SIGNER_LOG=debug navio-signerd
```

Most config errors print a clear message and exit non-zero.

### "already running" / lockfile contention

- The PID-based lockfile lives at
  `$XDG_DATA_HOME/navio-signer/navio-signer.lock`. It auto-clears if the holding
  PID is gone. If you see a stale lock, check it manually:

  ```sh
  cat "$XDG_DATA_HOME/navio-signer/navio-signer.lock"
  ps -p <PID>
  # if no process, delete the file and retry
  ```

---

## Still stuck

- Set `NAVIO_SIGNER_LOG=debug` for one cycle and capture both log files.
- Open an issue at <https://github.com/mxaddict/navio-signer/issues> with:
  - The relevant 50–100 log lines (redact tokens / passphrases).
  - The `navio-signer status` output for the affected run.
  - `navio-signer --version`.
