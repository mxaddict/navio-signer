# Signing key bootstrap

One-time setup for the three signing pipelines (linux GPG, Windows Authenticode,
macOS Developer ID + notarization).

All steps assume **macOS** as the signer host. Hardware-backed key storage
(YubiKey, Secure Enclave) is recommended for production but optional; this guide
covers software keys.

> Plaintext secrets (PKCS#12 password, GPG passphrase if cached) live on local
> disk. Run on an **encrypted volume** (FileVault on by default in recent macOS)
> and keep secret files at **0600**.

---

## 1. Apple Developer ID + notarization

### 1.1 Enroll + obtain the certificate

1. Enroll in the Apple Developer Program ($99/yr) under the organization account
   (`team-id` will appear in the developer portal).
2. In the developer portal, create a **Developer ID Application** certificate.
   Follow the CSR flow with **Keychain Access → Certificate Assistant → Request
   a Certificate From a Certificate Authority** to produce a CSR that contains a
   private key already in your login keychain.
3. Download the issued `.cer`, double-click to install. Verify:

   ```sh
   security find-identity -p codesigning -v
   ```

   You should see a line like:

   ```
   1) ABCDEF1234567890ABCDEF1234567890ABCDEF12 "Developer ID Application: Your Name (TEAMID)"
   ```

   Copy the full quoted string — that goes into `signing.macos.identity` in
   `config.toml`.

### 1.2 Notarization credentials

Notarization uses an **app-specific password** (not your Apple ID password):

1. Sign in at <https://appleid.apple.com> → **Sign-In and Security** →
   **App-Specific Passwords** → generate one labeled "navio-signer".
2. Store the credential in the macOS keychain under a profile name:

   ```sh
   xcrun notarytool store-credentials "navio-notary" \
       --apple-id you@example.com \
       --team-id TEAMID \
       --password "<app-specific-password>"
   ```

3. Verify the profile works against a dummy zip:

   ```sh
   xcrun notarytool history --keychain-profile navio-notary
   ```

   This goes into `signing.macos.keychain_profile`.

### 1.3 Why no stapling?

navio ships bare Mach-O binaries, not `.dmg` / `.pkg` / `.app` bundles. There's
no container for `xcrun stapler` to write a notarization ticket into. Gatekeeper
instead pulls the ticket from Apple's CDN at first launch — this means binaries
must reach end-user machines online for the first run, but no extra signer-side
step is needed.

---

## 2. GnuPG (linux + SHA256SUMS)

### 2.1 Install

```sh
brew install gnupg pinentry-mac
```

Configure `pinentry-mac` so the passphrase prompts can use the macOS keychain:

```sh
mkdir -p ~/.gnupg
echo "pinentry-program $(brew --prefix)/bin/pinentry-mac" > ~/.gnupg/gpg-agent.conf
gpgconf --kill gpg-agent
```

### 2.2 Generate or import the release key

To generate fresh:

```sh
gpg --full-generate-key
# Select: (1) RSA and RSA, 4096 bits, 0 = never expires (or 1y if you rotate),
# Real name: navio-core release signer
# Email: releases@navcoin.org    # or whatever the project uses
```

To import an existing private key:

```sh
gpg --import path/to/private.asc
```

### 2.3 Find the fingerprint + publish the public part

```sh
gpg --list-secret-keys --keyid-format=long
# sec   rsa4096/0123456789ABCDEF 2026-05-27 [SC]
#       0123456789ABCDEF0123456789ABCDEF01234567
# uid                 navio-core release signer <...>
```

The full 40-char fingerprint goes into `signing.linux.gpg_key_id`.

Export and publish the public key:

```sh
gpg --armor --export 0123456789ABCDEF0123456789ABCDEF01234567 > navio-release.pub.asc
```

Commit `navio-release.pub.asc` to the navio-core repo (or publish on
keys.openpgp.org) so end users can verify `SHA256SUMS.asc`.

### 2.4 Cache the passphrase (optional)

`navio-signer` runs `gpg --batch --yes` — a passphrase prompt would **fail the
run**, not block on a TTY. To avoid this either:

- Use an empty passphrase (acceptable on a dedicated signer box with FileVault),
  or
- Cache via gpg-agent with a long TTL:

  ```ini
  # ~/.gnupg/gpg-agent.conf
  default-cache-ttl 86400
  max-cache-ttl 604800
  ```

  Then warm the cache once interactively after each reboot:

  ```sh
  echo test | gpg --clearsign --local-user <FINGERPRINT> > /dev/null
  ```

---

## 3. Windows Authenticode (`osslsigncode`)

### 3.1 Install osslsigncode

```sh
brew install osslsigncode
osslsigncode --version
```

### 3.2 Obtain a code-signing certificate

Code-signing certs come from a commercial CA (Sectigo, DigiCert, SSL.com, etc.).
Options:

- **OV (Organization Validation)** — file-based PKCS#12, cheapest, takes 1–3
  business days. Works with `osslsigncode -pkcs12 / -pass`. Browsers show
  "Unknown Publisher" until SmartScreen reputation builds up.
- **EV (Extended Validation)** — hardware token (FIPS 140-2 USB key). Immediate
  SmartScreen reputation. Does **not** work with the current
  `osslsigncode -pkcs12` flow without extra PKCS#11 plumbing — out of scope
  here.

This guide assumes OV.

### 3.3 Convert + store the cert

CAs typically deliver a `.cer` (public) + private key in the browser that
generated the CSR. Export both to a single `.p12`:

```sh
# From Keychain Access GUI: right-click the imported identity → Export
# → File Format: Personal Information Exchange (.p12)
# Pick a strong passphrase when prompted.

mv ~/Downloads/navio-codesign.p12 ~/keys/navio-codesign.p12
chmod 0600 ~/keys/navio-codesign.p12
```

Put the absolute path in `signing.windows.pkcs12_path` and the passphrase in
`signing.windows.pkcs12_password`.

### 3.4 Verify

Test against a throwaway `.exe`:

```sh
osslsigncode sign \
    -pkcs12 ~/keys/navio-codesign.p12 \
    -pass "$(security find-generic-password -w -s navio-codesign)" \
    -t http://timestamp.sectigo.com \
    -in /tmp/hello.exe \
    -out /tmp/hello-signed.exe
osslsigncode verify /tmp/hello-signed.exe
```

`verify` should report `Signature verification: ok`.

---

## 4. Final sanity check

After all three pipelines are configured:

```sh
navio-signer poll
navio-signer status
# Pick a recent run_id from the output:
navio-signer fetch   <RUN_ID>
navio-signer verify  <RUN_ID>
navio-signer sign    <RUN_ID>
# Sign step should print: GPG-signed N tarballs, Authenticode-signed M
# binaries, codesigned+notarized K Mach-O binaries.
navio-signer publish <RUN_ID>
# Publish step uploads to a draft-equivalent release; check it on the
# GH repo before merging PR nav-io/navio-core#262 to retire the
# guix.yml publish step.
```

See [TROUBLESHOOTING.md](TROUBLESHOOTING.md) for what to do when a step fails.
