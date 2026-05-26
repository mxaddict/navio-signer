# launchd agent

The shipped launchd plist runs `navio-signerd` as a per-user agent that
auto-starts at login and restarts on crash.

## Install

```sh
cargo install --path . --root /usr/local
./contrib/install.sh
```

`install.sh` does the following:

1. Resolves `navio-signerd` (either via `--bin /abs/path` or `command -v`).
2. Substitutes `__BIN__` and `__HOME__` placeholders in the template at
   `contrib/launchd/sh.navio.signer.plist`.
3. Writes the rendered plist to `~/Library/LaunchAgents/sh.navio.signer.plist`.
4. Creates the log directory `~/Library/Logs/navio-signer/`.
5. `launchctl unload`s any prior copy, then `launchctl load`s the new one.

After install, the daemon is running. Verify:

```sh
launchctl list | grep sh.navio.signer
```

A `0` (or `-`) PID and `0` exit status means the agent is loaded and the last
invocation succeeded. A repeating non-zero PID means it crashed and launchd is
throttling restarts (`ThrottleInterval = 30s`).

## Log paths

| Stream | Path                                     |
| ------ | ---------------------------------------- |
| stdout | `~/Library/Logs/navio-signer/stdout.log` |
| stderr | `~/Library/Logs/navio-signer/stderr.log` |

Tail both:

```sh
tail -f ~/Library/Logs/navio-signer/*.log
```

`navio-signer` writes structured tracing output (one event per line). Use `grep`
/ `jq` / the `bunyan` CLI as appropriate — see
[TROUBLESHOOTING.md](TROUBLESHOOTING.md) for common patterns.

## Stop / restart

```sh
# Stop (one-shot, until next login or load)
launchctl unload ~/Library/LaunchAgents/sh.navio.signer.plist

# Start again
launchctl load ~/Library/LaunchAgents/sh.navio.signer.plist

# Force-trigger one daemon iteration without waiting for KeepAlive
launchctl kickstart -k gui/$(id -u)/sh.navio.signer
```

## Uninstall

```sh
launchctl unload ~/Library/LaunchAgents/sh.navio.signer.plist
rm ~/Library/LaunchAgents/sh.navio.signer.plist
rm -rf ~/Library/Logs/navio-signer
```

(State DB at `~/Library/Application Support/navio-signer/builds.db` is preserved
— wipe explicitly if you want a clean slate.)

## Customizing the plist

The shipped template is intentionally minimal. Common adjustments:

| Need                   | Edit                                                                                         |
| ---------------------- | -------------------------------------------------------------------------------------------- |
| Higher log verbosity   | Add `<key>EnvironmentVariables</key>` entries for `NAVIO_SIGNER_LOG`.                        |
| Different `PATH`       | Edit the `PATH` value under `EnvironmentVariables` (currently includes `/opt/homebrew/bin`). |
| Don't restart on crash | Remove the `<key>KeepAlive</key><true/>` pair.                                               |
| Custom config path     | Add `NAVIO_SIGNER_CONFIG` under `EnvironmentVariables`.                                      |

After editing, rerun `./contrib/install.sh` (it `unload`s + `load`s for you) or
`launchctl unload && launchctl load` manually.

## Why a LaunchAgent and not a LaunchDaemon?

- Signing keys live in the **login keychain**, which is only unlocked after
  login. A LaunchDaemon (system-scope, starts pre-login) cannot reach those
  credentials without extra keychain unlock steps.
- `xcrun notarytool` uses the user's keychain profile.
- GPG agent runs per-user.

If you need machine-scope persistence anyway, move the plist to
`/Library/LaunchDaemons/`, change `WorkingDirectory` to an absolute path, move
keychains/keys to the system keychain or a dedicated path, and adjust ownership.
Out of scope for the default install.
