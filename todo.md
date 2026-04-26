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

## Phase 2 — Dependency refresh ✅ done 2026-04-26

- [x] Bumped `clap` 4.3 → 4.6 — `--help` and all subcommand parsing verified.
- [x] Bumped `toml` 0.8 → 1.1 — round-trip of `git-agecrypt.toml` covered by `config::app::tests::load_save_round_trip_preserves_entries` and the e2e suite.
- [x] Bumped `thiserror` 1 → 2 — no source changes needed; the `From`-based variants in `src/git.rs` and `src/config/mod.rs` are source-compatible.
- [x] Bumped `blake3` 1.3 → 1.8, `regex` 1.8 → 1.12, `anyhow` 1.0.52 → 1.0.102, `serde` 1.0.133 → 1.0.228, `log` 0.4.14 → 0.4.29, `env_logger` 0.11.3 → 0.11.10, `assert_fs` 1.0 → 1.1; `assert_matches` already at 1.5.
- [x] Bumped dev-deps `rstest` 0.18 → 0.26 and `duct` 0.13 → 1.1; ran `cargo update -p futures-util` to drop the yanked 0.3.30 (now 0.3.32). `cargo audit` reports only the whitelisted RSA/Marvin advisory.
- [x] Regenerated `Cargo.lock`.
- [x] Bumped `edition` 2021 → 2024. `cargo fix --edition` ran clean once the new clippy uninlined-format-args were applied. `cargo clippy --all --tests -- -D warnings` is green.

### Coverage uplift performed alongside this phase

- [x] Added 45 new `#[cfg(test)]` unit tests across `src/age.rs`, `src/config/app.rs`, `src/config/age_identities.rs`, `src/config/git.rs`, `src/cli/args.rs` (clap-derive `From` conversions, parsing edge cases) and 11 new integration tests in `tests/e2e.rs` (textconv-on-plaintext, recipient-only / path-only remove paths, fresh-repo status / list, broken-identity marker, deinit-without-init).
- [x] Total tests: **20 → 76** (50 unit + 26 e2e).
- [x] Total coverage: **78.53 % → 88.67 % regions / 84.79 % → 92.36 % lines / 84.11 % → 93.83 % functions** (per `cargo llvm-cov --summary-only`).

## Phase 3 — Code quality ✅ done 2026-04-26

- [x] Run `cargo clippy --fix` for the 7 `uninlined_format_args` lints in `src/age.rs` and `src/cli/public.rs`.
- [x] Delete unused `Repository::get_config` trait method in `src/git.rs:49` (kept as `#[cfg(test)]` inherent method).
- [x] Replace `.unwrap()` on `contents.as_blob()` in `src/git.rs:107` with a typed error for non-blob tree entries.
- [x] Enforce `cargo clippy -D warnings` in CI (via `.github/workflows/ci.yml`).
- [x] Replaced the 3 `panic!("Misconfigured config parser")` arms with `unreachable!` calls + comments pointing at the responsible clap `ArgGroup`. Discovered the `RemoveConfig` arm was actually reachable (`config remove` with no flags panicked at runtime); added a new `target` ArgGroup with `required(true)` so clap rejects this at parse time. Regression test in `src/cli/args.rs::tests::remove_config_requires_target`.
- [x] Audited every `to_string_lossy()`:
    - `ctx.rs::get_sidecar` and `ctx.rs::current_exe` and `age.rs::{load_identities, validate_identity}` — switched to `Path::to_str()` with explicit UTF-8 errors. Lossy conversion here would have silently corrupted sidecar filenames or filter command paths.
    - `config/app.rs:58, 105` — kept lossy (display-only error messages and `config list` output).
    - `config/age_identities.rs:130` — was already inside `#[cfg(test)]`.
- [x] Quote the executable path in filter commands. New `shell_quote()` helper in `src/cli/public.rs` wraps the exe path in double quotes and escapes `\`, `"`, `$`, `` ` ``. New e2e regression `init_quotes_exe_path_in_filter_commands` plus 4 unit tests for the helper.
- [x] Made `// Decrypt files for diff` in `src/cli/args.rs:179` a doc comment (`///`) so clap renders it.

### Test coverage uplift this phase

- [x] Added 14 new tests (13 unit + 1 e2e), bringing the total from 76 → **90 (63 unit + 27 e2e)**.
- [x] New ctx.rs unit tests exercise `get_sidecar`, `store_sidecar`/`load_sidecar` round-trip, idempotent `remove_sidecar_files`, and (on Unix) the non-UTF8 path rejection path via a `FakeRepo` trait stub.
- [x] New args.rs test asserts bare `config remove` is rejected by clap.
- [x] New e2e `init_quotes_exe_path_in_filter_commands` verifies the stored filter command starts with a double quote and has a balanced pair.
- [x] Coverage: 88.67 % → 89.00 % regions / 84.79 % → 91.23 % lines / function coverage 93.83 % → 90.53 % (dip is from `FakeRepo`'s `unimplemented!` test stubs — region/line metrics still rise).

## Phase 4 — User-visible typos ✅ done 2026-04-26

- [x] `recepients` → already corrected; no occurrence remaining in `src/age.rs`.
- [x] `Coldn't` → `Couldn't` in `src/config/app.rs::AppConfig::save`.
- [x] `follwing files doesn't exist` → `following files don't exist` in `src/config/app.rs::AppConfig::add`.
- [x] `Couldn not` → already corrected; the surviving wording in `src/git.rs::get_file_contents` is `Could not determine repository head`.
- [x] `Looking for saved has information` → `Looking for saved hash information` in `src/cli/internal.rs::clean`.

## Phase 5 — Test coverage ✅ done 2026-04-26 (now 89.96 % lines / 89.09 % regions)

- [x] Added 15 end-to-end tests in `tests/e2e.rs` exercising the real binary against ephemeral git repos, covering init/deinit, config add/list/remove for both identities and recipients, the clean/smudge/textconv pipeline, the hash short-circuit, and invalid-input error paths.
- [x] Regression test for the `deinit` `filter.*` typo now in place (`deinit_removes_both_filter_and_diff_sections`).
- [x] Round-trip tests for `age::{encrypt, decrypt, validate_public_keys, validate_identity}` are exercised transitively by the E2E suite (every command path goes through them).
- [x] Sandboxed Docker harness at `Dockerfile.test` + `scripts/test-docker.sh` + `just docker-test`, producing `coverage/{summary.txt,lcov.info,html/index.html,test-output.txt}` on the host.
- [x] Direct unit tests for `config::AppConfig` low-level paths added in Phase 2 (`load_save_round_trip_preserves_entries`, `load_returns_error_on_malformed_toml`, `add_dedups_within_single_call`, `get_public_keys_strips_repo_prefix`, …). `config/app.rs` region coverage 70.83 % → **96.73 %**.
- [x] Introduced a `TestContext` (in-process `Context` impl) in `src/cli/internal.rs::tests`. Drives `get_content` directly across all four branches (saved sidecar fast-path, fresh encrypt, HEAD-decrypts-to-same-plaintext reuse, HEAD-decrypts-to-different re-encrypt). `cli/internal.rs` region coverage 73.85 % → **82.51 %**.
- [x] Wired `cargo llvm-cov report --fail-under-lines 80` into the CI `coverage` job as a hard gate. Headline currently sits at ~90 %, so this catches genuine regressions without blocking minor fluctuations.

### CI coverage UX additions

- [x] `coverage` job in `.github/workflows/ci.yml` now writes the `cargo llvm-cov report --summary-only` table into `$GITHUB_STEP_SUMMARY` so the per-file table renders directly on the workflow run page (no artifact download required).
- [x] On `main` pushes, the same job writes a shields.io endpoint JSON (`{"schemaVersion":1,"label":"coverage","message":"<n>%","color":"brightgreen|green|yellowgreen|yellow|red"}`) and force-pushes it to an orphan `coverage-badge` branch. Pull-request runs do not refresh the badge.
- [x] README references the badge via `https://img.shields.io/endpoint?url=https://raw.githubusercontent.com/bartei/git-agecrypt/coverage-badge/coverage.json` — no Codecov / external service involved, no PAT required (uses `GITHUB_TOKEN` with `contents: write` scoped to the coverage job).

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
- [x] `shell.nix` no longer carries a custom `grcov` derivation — `pkgs.grcov` is in nixpkgs (0.9.x), so the deprecated `cargoSha256` field went away with the derivation. The dev shell now also surfaces `cargo-llvm-cov` and `cargo-audit` so the local just-targets work out of the box.
- [x] `flake.lock` refreshed (nixpkgs 2024-03 → 2026-04, flake-utils 2024-02 → 2024-11). `nix flake check`, `nix build .#default`, and `nix develop --command …` all succeed locally. Also fixed the deprecated `overlay` flake output to `overlays.default`.
- [x] Added `rust-toolchain.toml` pinning channel `1.86` (edition-2024 needs 1.85, but transitive `icu_*` crates pulled via `idna` push the effective MSRV up). Declared `package.rust-version = "1.86"` in `Cargo.toml`. All eight `dtolnay/rust-toolchain@stable` references across `ci.yml`, `audit.yml`, and `release.yml` updated to `@1.86` so CI and local dev share the toolchain (no clippy-lint drift).
- [ ] Document the per-file process-spawn limitation more prominently in `README.md`.
- [ ] Consider implementing the [git long-running filter protocol](https://git.kernel.org/pub/scm/git/git.git/tree/Documentation/technical/long-running-process-protocol.txt) to amortize startup cost across many files (deferred — design work, not sanitization).