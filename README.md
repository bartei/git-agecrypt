# git-agecrypt

[![CI](https://github.com/bartei/git-agecrypt/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/bartei/git-agecrypt/actions/workflows/ci.yml)
[![Audit](https://github.com/bartei/git-agecrypt/actions/workflows/audit.yml/badge.svg?branch=main)](https://github.com/bartei/git-agecrypt/actions/workflows/audit.yml)
[![Coverage](https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/bartei/git-agecrypt/coverage-badge/coverage.json)](https://github.com/bartei/git-agecrypt/actions/workflows/ci.yml)
[![Latest release](https://img.shields.io/github/v/release/bartei/git-agecrypt?sort=semver&display_name=tag)](https://github.com/bartei/git-agecrypt/releases/latest)
[![Downloads](https://img.shields.io/github/downloads/bartei/git-agecrypt/total)](https://github.com/bartei/git-agecrypt/releases)
[![License: MPL-2.0](https://img.shields.io/github/license/bartei/git-agecrypt)](./LICENSE)
[![Rust](https://img.shields.io/badge/rust-stable-orange.svg?logo=rust)](https://www.rust-lang.org)

Transparent file-level encryption for files in a git repository, powered by [age](https://age-encryption.org). The plaintext stays in your working tree; the ciphertext is what travels through `git add`, `git push`, and ends up in the remote.

`git-agecrypt` is a modern, portable replacement for [git-crypt](https://github.com/AGWA/git-crypt): same workflow, but using age instead of GPG. Recipients can be age x25519 keys, OpenSSH ed25519 / RSA keys, or any [age plugin](https://github.com/FiloSottile/awesome-age) recipient (e.g. YubiKey PIV via `age-plugin-yubikey`).

## Features

- **Encryption is transparent.** Once configured, regular `git add` / `git commit` / `git push` / `git pull` / `git diff` work as usual — files are encrypted on the way into the index and decrypted on checkout.
- **Multi-recipient.** Encrypt a file to any number of recipients; any single matching identity decrypts it.
- **Per-path policy.** Different files can be encrypted to different recipient sets, configured in a single committed `git-agecrypt.toml`.
- **Key rotation friendly.** Add or remove recipients without rewriting history; the next commit re-encrypts only what changed.
- **Hardware key support.** Use age plugin recipients to keep the long-lived secret on a YubiKey or another secure element.
- **Stable ciphertext.** Re-running `git status` / `git add` against an unchanged plaintext doesn't churn the encrypted blob — a blake3 sidecar makes the encrypted output deterministic relative to the working copy.

## Installation

### Pre-built binaries

Each [release](https://github.com/bartei/git-agecrypt/releases/latest) ships archives for:

| Platform | Archive |
|---|---|
| Linux x86_64 (glibc) | `git-agecrypt-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz` |
| Linux x86_64 (musl, static) | `git-agecrypt-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz` |
| macOS Intel | `git-agecrypt-vX.Y.Z-x86_64-apple-darwin.tar.gz` |
| macOS Apple Silicon | `git-agecrypt-vX.Y.Z-aarch64-apple-darwin.tar.gz` |
| Windows x86_64 | `git-agecrypt-vX.Y.Z-x86_64-pc-windows-msvc.zip` |

```console
# Example: Linux x86_64 musl, install to ~/.local/bin
$ curl -L https://github.com/bartei/git-agecrypt/releases/latest/download/git-agecrypt-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz \
    | tar -xz -C ~/.local/bin
$ git-agecrypt --version
```

### From source (cargo)

```console
$ cargo install --git https://github.com/bartei/git-agecrypt
```

Or clone and build:

```console
$ git clone https://github.com/bartei/git-agecrypt
$ cd git-agecrypt
$ cargo install --path .
```

### Nix

```console
$ nix profile install github:bartei/git-agecrypt
```

A development shell (`pkg-config`, `libgit2`, `cargo-limit`, `cargo-watch`, `just`, `grcov`) is available via `nix develop` or `nix-shell`.

### Verify the install

```console
$ git-agecrypt --help
$ git-agecrypt --version
```

The binary should be discoverable on `PATH`. `git-agecrypt init` records the absolute path to the executable in the repo's `.git/config`, so once a repo is initialized you can move the binary, but you'll need to re-run `init` afterwards.

## Quick start (5 minutes)

This walkthrough encrypts `secrets/api-token` for yourself, using a fresh age x25519 keypair.

```console
# 1. Generate a personal age identity. Treat the resulting file like an SSH
#    private key — it's the only thing that can decrypt the repo's secrets.
$ age-keygen -o ~/.config/age/personal.key
Public key: age1qz5y…0p7w
$ chmod 600 ~/.config/age/personal.key

# 2. Inside your git repo, install the filter integration.
$ cd ~/work/my-project
$ git-agecrypt init

# 3. Tell git-agecrypt where YOUR private key lives. This is local-only;
#    it goes into .git/config, not into a tracked file.
$ git-agecrypt config add -i ~/.config/age/personal.key

# 4. Create the file you want to encrypt and register a recipient for it.
$ mkdir -p secrets
$ printf 'super-secret-token\n' > secrets/api-token
$ git-agecrypt config add \
    -r age1qz5y…0p7w \
    -p secrets/api-token

# 5. Tell git which paths the filter applies to. .gitattributes is committed.
$ echo 'secrets/* filter=git-agecrypt diff=git-agecrypt' >> .gitattributes

# 6. Commit. The blob in git is encrypted; your working copy is plaintext.
$ git add .gitattributes git-agecrypt.toml secrets/api-token
$ git commit -m "encrypted api-token"

# 7. Verify: this prints ciphertext (starts with "age-encryption.org/v1").
$ git show HEAD:secrets/api-token
```

After cloning the repo on another machine, run `git-agecrypt init` once and `git-agecrypt config add -i <path-to-private-key>` — the recipients in `git-agecrypt.toml` are already there. `git checkout` will decrypt back to plaintext automatically.

## Commands

All commands print contextual help with `git-agecrypt <command> --help`.

### `git-agecrypt init`

Wires `git-agecrypt` into the current repository as a clean / smudge / textconv driver in `.git/config`. Idempotent — safe to re-run after moving the binary.

```console
$ git-agecrypt init
```

### `git-agecrypt deinit`

Removes the filter and diff configuration this tool added, and clears the per-file ciphertext cache under `.git/git-agecrypt/`. **Files committed encrypted stay encrypted in history** — `deinit` only removes the local integration, not the encryption itself.

```console
$ git-agecrypt deinit
```

### `git-agecrypt status`

Prints the currently configured identities (decryption keys) and recipients (encryption keys). Use this to confirm a repo is set up correctly.

```console
$ git-agecrypt status
The following identities are currently configured:
    ✓ /home/alice/.config/age/personal.key

The following recipients are configured:
    secrets/api-token: age1qz5y…0p7w
    secrets/db.env:    age1qz5y…0p7w
    secrets/db.env:    age1jrnk…2qzp   # bob's key
```

A `✓` mark means the identity file exists and parses; a `⨯` mark means it's misconfigured (file missing, wrong permissions, not a valid age key, etc.).

### `git-agecrypt config add -i <path>` — register a decryption identity

Tells `git-agecrypt` where to find one of *your* private keys. Stored in `.git/config` under `git-agecrypt.config.identity`, so it's per-clone and never committed. You can register multiple identities; any one that matches will be used during decryption.

```console
# Native age x25519 secret key file
$ git-agecrypt config add -i ~/.config/age/personal.key

# An OpenSSH ed25519 key already used for SSH auth
$ git-agecrypt config add -i ~/.ssh/id_ed25519

# A YubiKey-backed age plugin identity stub
$ git-agecrypt config add -i ~/.config/age/yubikey-stub.txt
```

### `git-agecrypt config remove -i <path>`

Removes a previously registered identity. The key file itself is not deleted.

```console
$ git-agecrypt config remove -i ~/.config/age/personal.key
```

### `git-agecrypt config list -i`

Lists registered identities, each annotated with whether it currently parses as a valid age identity.

```console
$ git-agecrypt config list -i
The following identities are currently configured:
    ✓ /home/alice/.config/age/personal.key
    ✓ /home/alice/.ssh/id_ed25519
```

### `git-agecrypt config add -r <recipient> -p <path>...` — add an encryption recipient

Registers one or more **public** keys (recipients) that should be able to decrypt one or more **paths**. Both `-r` and `-p` accept multiple values; you can also repeat the command per recipient. The mapping lives in a committed `git-agecrypt.toml` at the repo root, so collaborators inherit it on clone.

Recipients can be:

- An age native public key: `age1…`
- An OpenSSH `ssh-ed25519` or `ssh-rsa` line: typically `cat ~/.ssh/id_ed25519.pub`
- An age plugin recipient: e.g. the `age1yubikey1…` line emitted by `age-plugin-yubikey`

```console
# Encrypt one file to one recipient
$ git-agecrypt config add \
    -r "$(cat ~/.ssh/id_ed25519.pub)" \
    -p secrets/api-token

# Encrypt several files to the same recipient
$ git-agecrypt config add \
    -r age1qz5y…0p7w \
    -p secrets/api-token secrets/db.env config/prod.env

# Encrypt one file to several recipients (alice, bob, ci)
$ git-agecrypt config add \
    -r age1qz5y…0p7w \
    -r age1jrnk…2qzp \
    -r age1ci8m…lkpw \
    -p secrets/api-token
```

After the first `config add`, edit `.gitattributes` (a regular committed file) so git knows which paths use this filter:

```gitattributes
secrets/**          filter=git-agecrypt diff=git-agecrypt
config/*.env        filter=git-agecrypt diff=git-agecrypt
**/*.secret.yaml    filter=git-agecrypt diff=git-agecrypt
```

> Filters are applied to files, not directories — write `secrets/**` (or list specific files) rather than `secrets/`.

### `git-agecrypt config remove -r <recipient>` — remove a recipient

Drop a recipient from one or more paths, or from every path it currently appears in. Removing a recipient does **not** rewrite history or re-encrypt existing blobs; it takes effect on the next change to each affected file.

```console
# Drop bob from secrets/api-token only
$ git-agecrypt config remove \
    -r age1jrnk…2qzp \
    -p secrets/api-token

# Drop bob entirely (every path he was a recipient of)
$ git-agecrypt config remove -r age1jrnk…2qzp

# Clear all recipients of a path (e.g. before reassigning)
$ git-agecrypt config remove -p secrets/api-token
```

### `git-agecrypt config list -r`

Prints the path → recipient mapping currently in `git-agecrypt.toml`.

```console
$ git-agecrypt config list -r
The following recipients are configured:
    secrets/api-token: age1qz5y…0p7w
    secrets/api-token: age1jrnk…2qzp
    secrets/db.env:    age1qz5y…0p7w
```

### Internal subcommands

`git-agecrypt clean`, `smudge`, and `textconv` are invoked by git itself via the filter wiring set up by `init`. You should not need to call them directly. They are hidden from `--help` to keep the user-facing CLI small.

## Common workflows

### Onboard a new collaborator (Bob)

Bob has just shared his age public key with you. To grant him access to a secret:

```console
# 1. Add Bob's recipient to every file he should be able to decrypt.
$ git-agecrypt config add \
    -r age1jrnk…2qzp \
    -p secrets/api-token secrets/db.env

# 2. Touch each affected file so the next commit re-encrypts to the
#    expanded recipient set. (`cat … > …` rewrites mtime in place.)
$ for f in secrets/api-token secrets/db.env; do cp "$f" "$f.tmp" && mv "$f.tmp" "$f"; done

# 3. Commit and push.
$ git add git-agecrypt.toml secrets/api-token secrets/db.env
$ git commit -m "grant bob access to api-token and db.env"
$ git push
```

On Bob's side:

```console
$ git clone <repo>
$ cd <repo>
$ git-agecrypt init
$ git-agecrypt config add -i ~/.config/age/bob.key
$ git checkout -- secrets/   # smudge filter decrypts on checkout
```

### Off-board a collaborator (revoke Bob)

```console
$ git-agecrypt config remove -r age1jrnk…2qzp
# Re-touch every file so it gets re-encrypted to the reduced recipient set.
$ git ls-files | xargs -I {} sh -c 'grep -q git-agecrypt .gitattributes && cp "{}" "{}.tmp" && mv "{}.tmp" "{}"' || true
$ git add -A && git commit -m "revoke bob"
$ git push
```

> **Important:** `git-agecrypt` controls the *current* state of the repository, not its history. If Bob ever cloned the repo, he still holds copies of the encrypted blobs as they were at clone time, and his identity can still decrypt them. After revoking, **rotate the underlying secrets** (e.g. issue a new API token, change the DB password) — that is the actual revocation.

### Use a YubiKey via `age-plugin-yubikey`

```console
# One-time setup — generates a hardware-backed age identity slot.
$ age-plugin-yubikey
# Pick "Generate a new identity"; note the printed recipient (age1yubikey1…)
# and the saved identity stub file (e.g. ~/.config/age/yubikey-stub.txt).

# Register the stub locally as your identity.
$ git-agecrypt config add -i ~/.config/age/yubikey-stub.txt

# Register the printed recipient so files get encrypted to your YubiKey.
$ git-agecrypt config add -r age1yubikey1… -p secrets/api-token
```

Decryption now requires the YubiKey to be plugged in and (depending on slot config) touched.

### CI/CD: a deploy key that can read secrets

Generate a dedicated keypair for CI, register the public key as a recipient, and inject the private key into the CI environment (e.g. as a base64-encoded secret).

```console
# Local: generate the CI keypair
$ age-keygen -o ./ci.key
Public key: age1ci8m…lkpw
$ git-agecrypt config add -r age1ci8m…lkpw -p $(git ls-files secrets/)

# CI workflow (pseudo-code):
#   echo "$AGE_CI_KEY" > /tmp/ci.key && chmod 600 /tmp/ci.key
#   git-agecrypt init
#   git-agecrypt config add -i /tmp/ci.key
#   git checkout -- secrets/
```

### Encrypt a brand new file in a fresh clone

```console
$ git-agecrypt init
$ git-agecrypt config add -i ~/.config/age/personal.key
$ printf 'shh\n' > secrets/new-token
$ git-agecrypt config add -r age1qz5y…0p7w -p secrets/new-token
$ echo 'secrets/new-token filter=git-agecrypt diff=git-agecrypt' >> .gitattributes
$ git add .gitattributes git-agecrypt.toml secrets/new-token
$ git commit -m "encrypted new-token"
```

### Inspect what's actually in git

```console
# What does git see for this file? (Should be age ciphertext.)
$ git show HEAD:secrets/api-token

# Diff between commits decrypts via the textconv filter and shows plaintext.
$ git log -p secrets/api-token

# The working copy is always plaintext.
$ cat secrets/api-token
```

### Tear down

```console
$ git-agecrypt deinit
```

This removes the filter wiring from `.git/config` and clears the local `.git/git-agecrypt/` cache. Tracked files (`.gitattributes`, `git-agecrypt.toml`) are not modified — delete them by hand if you want to fully un-encrypt the repo.

## How it works

`git-agecrypt` plugs into git's [smudge / clean / textconv filter mechanism](https://git-scm.com/book/en/v2/Customizing-Git-Git-Attributes). After `init`, the repo's `.git/config` contains:

```gitconfig
[filter "git-agecrypt"]
    required = true
    smudge   = /path/to/git-agecrypt smudge -f %f
    clean    = /path/to/git-agecrypt clean -f %f
[diff "git-agecrypt"]
    textconv = /path/to/git-agecrypt textconv
```

Git invokes these driver commands per file when `.gitattributes` opts the path in:

- **`clean`** runs on `git add`. Plaintext is read on stdin; ciphertext goes to stdout into the index.
- **`smudge`** runs on `git checkout`. Ciphertext from the index is read on stdin; plaintext is written into the working tree.
- **`textconv`** runs on `git diff` / `git log -p`. The encrypted file is decrypted on the fly so diffs show plaintext.

### Stable ciphertext via blake3 sidecars

Age encryption is non-deterministic: the same plaintext encrypted twice produces two different ciphertexts. Without mitigation, every `git status` / `git add` would create a fresh encrypted blob and show every encrypted file as modified. To avoid this, `clean` keeps two sidecar files per encrypted path under `.git/git-agecrypt/`:

- `<path>.hash` — blake3 hash of the most recent plaintext seen
- `<path>.age`  — the ciphertext last produced for that plaintext

When `clean` is invoked, it hashes the incoming plaintext. If the hash matches the saved one, the saved ciphertext is emitted verbatim — git sees no change. If the hash differs, `clean` decrypts the version currently in `HEAD` and compares it against the new plaintext; if they're equal, the HEAD ciphertext is reused. Only if both checks fail does the file get re-encrypted with fresh randomness.

`smudge` populates these sidecars too, so the next `clean` on the same plaintext short-circuits.

### Where things are stored

| Location | Contents | Tracked in git? |
|---|---|---|
| `git-agecrypt.toml` (repo root) | path → recipient mappings | **Yes** — committed and shared |
| `.gitattributes` | which paths use the filter | **Yes** — committed and shared |
| `.git/config` (filter wiring) | absolute path to the binary | No |
| `.git/config` (`[git-agecrypt "config"] identity`) | paths to *your* private keys | No |
| `.git/git-agecrypt/*.{hash,age}` | per-file ciphertext + plaintext-hash cache | No |

Encryption never needs the private key, so `git-agecrypt config add -r` works on a fresh clone before any identity is configured. Decryption of course does — that's what `config add -i` is for.

## Limitations

- The binary is re-executed once per file per git operation. Repos with thousands of encrypted files will see noticeable overhead. Implementing git's [long-running filter protocol](https://git.kernel.org/pub/scm/git/git.git/tree/Documentation/technical/long-running-process-protocol.txt) would amortize this, but isn't done yet.
- During encrypt / decrypt the whole file is loaded into memory. Don't use this for files that don't fit in RAM.
- Filters apply to files, not directories. Use `secrets/**`, not `secrets/`, in `.gitattributes`.
- Removing a recipient does not rewrite git history — old commits remain decryptable by the previous keys. Treat key revocation as a prompt to also rotate the underlying secrets.

## Known security advisories

The `age` crate transitively depends on `rsa` 0.9.x, which is affected by [RUSTSEC-2023-0071](https://rustsec.org/advisories/RUSTSEC-2023-0071) ("Marvin Attack" — potential key recovery through timing sidechannels). No upstream fix is currently available. This advisory only affects users who decrypt files for **SSH-RSA recipients**; users who only use x25519 (age native), ed25519 SSH keys, or age plugin recipients are not impacted.

The advisory is whitelisted in [`.cargo/audit.toml`](./.cargo/audit.toml) so CI's `cargo audit` job stays green; it will be removed as soon as upstream ships a fix.

## Contributing

Bug reports and PRs are welcome — open an issue at <https://github.com/bartei/git-agecrypt/issues>. Local development setup:

```console
$ just check          # fmt + clippy + tests
$ just docker-test    # full test suite + coverage in a sandboxed container
$ just coverage       # local coverage with cargo-llvm-cov
```

CI runs `cargo fmt --check`, `cargo clippy -D warnings`, the test suite on Linux / macOS / Windows, `cargo audit`, and a `cargo llvm-cov` coverage job. The coverage job:

- prints the per-file table on the run's **Summary** page (no need to download the artifact),
- uploads `lcov.info` / `coverage.json` / a per-file summary as a downloadable artifact,
- gates CI with `--fail-under-lines 80`,
- on `main`, refreshes the [coverage badge](#) by publishing a shields.io endpoint JSON to the orphan `coverage-badge` branch.

## License

[Mozilla Public License 2.0](./LICENSE).
