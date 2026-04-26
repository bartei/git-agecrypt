# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Common commands

The `justfile` is the canonical task runner. The `cargo l*` aliases come from [`cargo-limit`](https://github.com/alopatindev/cargo-limit) and are provided by the Nix dev shell — if `cargo-limit` is not installed, substitute the plain `cargo` subcommand.

- `just build` — `cargo lbuild` (debug build of the `git-agecrypt` binary)
- `just test` — `cargo ltest` (full test suite)
- `just clippy` — `cargo lclippy --all`
- `just fmt` — `cargo fmt --all`
- `just check` — runs `fmt`, `clippy`, then `test` (use this before declaring work done)
- `just watch [COMMAND]` — `cargo watch` loop, defaults to `ltest`
- `just covreport` — produces an HTML coverage report under `target/debug/coverage` (requires `coverage=1` set when building/testing and `llvm-tools-preview` installed via `just dev`)

Run a single test: `cargo test --all -- <test_name_substring>` (e.g. `cargo test -- test_get_file_contents`). Tests use `rstest` fixtures and `assert_fs` temp directories — they shell out to the real `git` binary, so `git` must be on `PATH`.

A reproducible dev shell is available via Nix: `nix develop` (flake) or `nix-shell` (legacy). It provides `pkg-config`, `openssl`, `libgit2`, `clang`, `just`, `grcov`, `cargo-limit`, `cargo-watch`.

## Architecture

`git-agecrypt` is a single binary that is invoked in two distinct modes by `cli::run` (`src/cli/app.rs`):

- **Public commands** (`init`, `deinit`, `status`, `config …`) — user-facing, mutate `.git/config` and `git-agecrypt.toml`.
- **Internal commands** (`clean`, `smudge`, `textconv`) — hidden from `--help`; invoked by git itself per file via the filter/diff configuration written by `init`. Each invocation reads from stdin (or a path for `textconv`) and writes to stdout.

### How encryption stays stable across git operations

Age encryption is non-deterministic, so naively re-encrypting on every `git status`/`git add` would churn the index. The `clean` filter (`src/cli/internal.rs`) avoids this by maintaining sidecar files under `.git/git-agecrypt/<encoded-path>.{hash,age}`:

1. blake3-hash the incoming plaintext.
2. If the hash matches the saved sidecar hash, return the previously-saved ciphertext verbatim (no re-encryption).
3. Otherwise, decrypt whatever is currently in `HEAD` for this path. If that plaintext matches the incoming plaintext, treat the HEAD ciphertext as canonical and store its hash + bytes — this is what handles fresh checkouts where no sidecar exists yet.
4. Only if both checks fail, re-encrypt with the configured recipients and refresh both sidecars.

`smudge` is the inverse: decrypt stdin → write plaintext to stdout, and pre-populate the sidecars so the next `clean` short-circuits. `textconv` decrypts a file on disk for `git diff`/`git log` output, falling back to verbatim content if it is not actually age-encrypted (working-copy case).

### Configuration split

Three distinct stores, each with a different lifetime:

- **`.git/config`** under `[filter "git-agecrypt"]` and `[diff "git-agecrypt"]` — the filter/textconv command lines written by `init` (uses absolute path to the current binary).
- **`.git/config`** under `[git-agecrypt "config"] identity = …` — paths to age **identities** (private keys). Per-checkout, never committed. Managed via the `GitConfig` container in `src/config/git.rs` wrapped by `AgeIdentities` in `src/config/age_identities.rs`.
- **`git-agecrypt.toml`** at the repo root — the path → recipients (public keys) mapping. Committed to the repo. Managed by `AppConfig` in `src/config/app.rs`.

Recipients can be x25519, ssh, or age plugin recipients (e.g. yubikey PIV stubs); identities can be any path readable by `age::cli_common::read_identities`. Plugin support is wired in `src/age.rs` via `RecipientPluginV1`.

### Trait layering (why everything is generic over `C: Context`)

- `git::Repository` (`src/git.rs`) abstracts libgit2 access — file contents from HEAD, multivar config get/set/list, and `remove_config_section` (which shells out to `git config --remove-section` because libgit2 lacks the operation). The single concrete implementation is `LibGit2Repository`.
- `ctx::Context` (`src/ctx.rs`) bundles a `Repository` with sidecar I/O, the current-exe path, and accessors for `AgeIdentities` and `AppConfig`. `ContextWrapper<R>` is the only impl.
- `config::Container` is the generic add/remove/list interface used by both `GitConfig` (for identities living in `.git/config`) and the recipient list. `Validated` gates inserts (e.g. an identity must parse as a valid age key before being stored).

The CLI command structs (`internal::CommandContext`, `public::CommandContext`) are generic over `Context`, so all I/O goes through these traits — this is the seam used by tests in `src/git.rs` (and where future test mocks for `Context` would plug in).

### Error handling

Two local error enums (`git::Error`, `config::Error`) both have `AlreadyExists` / `NotExist` / `Other(anyhow::Error)` variants and convert into each other. The CLI layer narrows these via `ensure_state` in `src/cli/public.rs`, which downgrades "already exists" / "doesn't exist" into `Ok(())` so that `init` and `deinit` are idempotent. Outside that helper, errors bubble up as `anyhow::Result`.

## Limitations to keep in mind when changing behavior

(From `README.md`, repeated here because they constrain design choices.) The binary is re-exec'd by git for every file on every operation — there is no long-running process protocol implementation yet. Whole files are loaded into memory during encrypt/decrypt. Filters apply per-file, so `.gitattributes` patterns must match files (use `secrets/**`, not `secrets/`).