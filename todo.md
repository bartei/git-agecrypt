# git-agecrypt — Sanitization TODO

Generated 2026-04-26 from a full repo review. Each item is independently actionable. Phases are ordered by risk and blast radius — finish a phase before the next.

## Phase 1 — Critical bug & security fixes ✅ done 2026-04-26

- [x] Fix `deinit` typo: `"fiter.git-agecrypt"` → `"filter.git-agecrypt"` in `src/cli/public.rs:35`.
- [x] Replace `Vec::dedup()` with set-based dedup in `AppConfig::add` (`src/config/app.rs:76`) so duplicate recipients are actually de-duplicated.
- [x] Remove the stray `println!("Error: {:?}", e)` in `src/age.rs:33` (writes to stdout from inside `clean` filter — corrupts git objects); replace with `log::error!`.
- [x] Bump `age` 0.10.0 → 0.11.x and migrate the `Decryptor::new` call site in `src/age.rs:21-36` (closes RUSTSEC-2024-0433 RCE; unlatches yanked crate). Pinned `i18n-embed-fl = 0.9.3` to work around upstream `fluent::concurrent` regression in 0.9.4.
- [x] Bump `git2` 0.18 → 0.20 (closes RUSTSEC-2026-0008 unsoundness; drags `idna` to fixed version, closes RUSTSEC-2024-0421).
- [x] Bumped `curve25519-dalek` 4.1.2 → 4.1.3 to close RUSTSEC-2024-0344 (timing variability) — was 5th vuln, missed in original Phase 1 list.
- [x] Re-run `cargo audit` — only the rsa/Marvin residual remains (no fix available); documented in README.md.

### Bonus fixes uncovered by E2E tests

- [x] `LibGit2Repository::get_file_contents` now returns `NotExist` (instead of opaque `Other`) when the repo has no commits yet (`UnbornBranch`). Without this, the very first `git add` of an encrypted file in a fresh repo would crash the clean filter.
- [x] Replaced `as_blob().unwrap()` with a proper `into_blob()` error path — addresses Phase 3 latent-panic item.

## Phase 2 — Dependency refresh

- [ ] Bump `clap` 4.3 → 4.6 and verify `--help` / subcommand surface still parses.
- [ ] Bump `toml` 0.8 → 1.1 and verify round-trip of `git-agecrypt.toml`.
- [ ] Bump `thiserror` 1 → 2 (mechanical migration in `src/git.rs` and `src/config/mod.rs`).
- [ ] Bump `blake3`, `regex`, `anyhow`, `serde`, `log`, `env_logger`, `assert_fs`, `assert_matches` to current minor/patch.
- [ ] Bump dev-deps `rstest` 0.18 → 0.26 and `duct` 0.13 → 1.1 (gets us off yanked `futures-util`).
- [ ] Regenerate `Cargo.lock` and commit it.
- [ ] Bump `edition` to `2024` and run `cargo fix --edition` if the bump is clean.

## Phase 3 — Code quality

- [x] Run `cargo clippy --fix` for the 7 `uninlined_format_args` lints in `src/age.rs` and `src/cli/public.rs`.
- [x] Delete unused `Repository::get_config` trait method in `src/git.rs:49` (kept as `#[cfg(test)]` inherent method).
- [x] Replace `.unwrap()` on `contents.as_blob()` in `src/git.rs:107` with a typed error for non-blob tree entries.
- [x] Enforce `cargo clippy -D warnings` in CI (via `.github/workflows/ci.yml`).
- [ ] Replace the 3 `panic!("Misconfigured config parser")` in `src/cli/args.rs:90,123,156` with `unreachable!` and a comment pointing at the clap `ArgGroup`.
- [ ] Audit every `to_string_lossy()` (`src/ctx.rs:94`, `src/age.rs:46,106`, `src/config/app.rs:64,108`) — switch to `OsStr`/`Path` APIs where feasible, error explicitly otherwise.
- [ ] Quote the executable path in filter commands (`src/cli/public.rs:23-29`) so installs into paths with spaces still work.
- [ ] Make `// Decrypt files for diff` in `src/cli/args.rs:179` a doc comment (`///`).

## Phase 4 — User-visible typos

- [ ] `recepients` → `recipients` in `src/age.rs:62`.
- [ ] `Coldn't` → `Couldn't` in `src/config/app.rs:52`.
- [ ] `follwing files doesn't exist` → `following files don't exist` in `src/config/app.rs:68`.
- [ ] `Couldn not` → `Could not` in `src/git.rs:95`.
- [ ] `Looking for saved has information` → `… hash …` in `src/cli/internal.rs:21`.

## Phase 5 — Test coverage (was 13.91 % lines → now 84.76 %)

- [x] Added 15 end-to-end tests in `tests/e2e.rs` exercising the real binary against ephemeral git repos, covering init/deinit, config add/list/remove for both identities and recipients, the clean/smudge/textconv pipeline, the hash short-circuit, and invalid-input error paths.
- [x] Regression test for the `deinit` `filter.*` typo now in place (`deinit_removes_both_filter_and_diff_sections`).
- [x] Round-trip tests for `age::{encrypt, decrypt, validate_public_keys, validate_identity}` are exercised transitively by the E2E suite (every command path goes through them).
- [x] Sandboxed Docker harness at `Dockerfile.test` + `scripts/test-docker.sh` + `just docker-test`, producing `coverage/{summary.txt,lcov.info,html/index.html,test-output.txt}` on the host.
- [ ] Add unit tests for `config::AppConfig` low-level paths (load/save TOML round-trip, path-prefix stripping) — current 70.83 % region coverage is via the CLI; would prefer direct unit coverage for the remaining branches.
- [ ] Introduce a `MockContext` to drive `cli::internal::CommandContext::{clean, smudge, textconv}` directly (currently covered only via subprocess invocations) — useful for faster iteration on filter logic.
- [ ] Wire `cargo llvm-cov report --fail-under-lines 80` into CI once a CI workflow exists.

## Phase 6 — Tooling, packaging, docs

- [x] CI workflow at `.github/workflows/ci.yml`: rustfmt, strict clippy (`-D warnings`), test matrix on Linux/macOS/Windows, coverage upload (Codecov + artifact), cargo-audit.
- [x] Release workflow at `.github/workflows/release.yml`: `workflow_dispatch` with patch/minor/major bump → tests → version bump in `Cargo.toml` + `Cargo.lock` → tag → GitHub Release with auto-generated notes → cross-platform binaries (Linux gnu/musl, macOS Intel/AS, Windows x86_64) uploaded as archives.
- [x] Daily scheduled audit workflow at `.github/workflows/audit.yml` so the README badge reflects current advisory state, not just per-PR validation.
- [x] `.github/dependabot.yml` — weekly cargo + github-actions + docker dependency updates with grouping.
- [x] `.cargo/audit.toml` whitelists RUSTSEC-2023-0071 (rsa Marvin, no fix available) so CI audit job exits clean.
- [x] README badges: CI, Audit, Coverage, Latest release, Downloads, License, Rust toolchain.
- [x] Documented in `README.md` the residual RSA/Marvin advisory.
- [x] Cleaned up `Repository::get_config` from the trait (kept as `#[cfg(test)]` inherent method to preserve test coverage of the libgit2 code path).
- [x] `cargo clippy --fix` applied — all 7 `uninlined_format_args` warnings cleared (CI now runs `-D warnings`).
- [ ] Remove the `Cargo.lock` line from `.gitignore` (this is a binary crate; lockfile should be tracked, and it already is).
- [ ] Update `shell.nix:18` from `cargoSha256` to `cargoHash` (deprecated field, fails on current nixpkgs).
- [ ] Refresh `flake.lock` (`nix flake update`) and verify `nix build` and `nix develop` still work.
- [ ] Add a `rust-toolchain.toml` pinning the MSRV (currently undeclared; CI uses 1.88).
- [ ] Document the per-file process-spawn limitation more prominently in `README.md`.
- [ ] Consider implementing the [git long-running filter protocol](https://git.kernel.org/pub/scm/git/git.git/tree/Documentation/technical/long-running-process-protocol.txt) to amortize startup cost across many files (deferred — design work, not sanitization).