# Dependency security sweep — design (2026-09-29)

## Goal

Clear every open RustSec finding against `main`, restore a green CI on `main`, and
triage the five open Dependabot PRs (#32, #33, #35, #36, #37) so the security-relevant
parts land and the noise is closed.

## Starting state (facts this design rests on)

- **`main` does not compile on the pinned toolchain.** PR #13 (merged 2026-06-14)
  bumped the transitive `rpassword` (via `age` → `cli-common`) to 7.5.2. `rpassword`
  7.5.x declares `rust-version = 1.85` but uses `if … && let` chains, which are only
  stable from **Rust 1.88**. Its incorrect MSRV metadata also defeats cargo's
  MSRV-aware resolver, which is why nothing warned. On 1.86, clippy, tests and
  coverage all fail with `E0658`. Every CI run on `main` since then has failed, and so
  has every Dependabot PR, whatever it changes. The red checks on those PRs tell us
  nothing about the PRs themselves.
- **The daily `Audit` workflow fails.** `cargo audit` on `main` reports:

  | Advisory | Crate | Kind | Path | Fixed by |
  |---|---|---|---|---|
  | RUSTSEC-2026-0204 | crossbeam-epoch 0.9.18 | vulnerability (invalid ptr deref in `fmt::Pointer`) | dev-only: assert_fs → globwalk → ignore → crossbeam-deque | ≥ 0.9.20 (**no open PR covers it**) |
  | RUSTSEC-2026-0190 | anyhow 1.0.102 | unsound (`Error::downcast_mut`) | direct dep | 1.0.103+ (PR #37) |
  | — | spin 0.9.8 | yanked | age → lazy_static | lockfile refresh |
  | RUSTSEC-2026-0173 | proc-macro-error2 2.0.1 | unmaintained | age 0.11 → i18n-embed-fl (build-time proc macro) | only `age` 0.12 |

- **Security fixes upstream that RustSec doesn't track:**
  - `age` 0.11.4 / 0.11.5 (backported alongside 0.12.0 / 0.12.1): identity and
    recipient files over the size limit now return an error instead of being
    silently truncated, and `ArmoredReader` now enforces the strict parsing profile
    (it rejects an empty final line and some truncated inputs that could make it hang).
    We pass repository contents through `ArmoredReader` in `src/age.rs`
    (the smudge/textconv decrypt path), so this fix applies to us directly.
  - `regex` 1.13.1 fixes unsound reverse-suffix/inner optimisations (incorrect
    match offsets). This is PR #32.
- **PR #36 is bogus.** It moves `dtolnay/rust-toolchain@1.86` to `@1.120`. Rust 1.120
  does not exist (current stable is around 1.98). Dependabot is treating a branch name
  in the action repo as a release. Beyond that, the action ref carries our MSRV pin on
  purpose, so bumping it automatically is always wrong.

## Approach

One branch, `fix/security-deps`, that supersedes the cargo PRs:

1. **Raise the MSRV from 1.86 to 1.88.** Change `Cargo.toml` `rust-version`,
   `rust-toolchain.toml`, and all eight `dtolnay/rust-toolchain@…` refs in `ci.yml`,
   `audit.yml` and `release.yml`.
2. **Run `cargo update`** (semver-compatible only). Because the edition-2024 resolver
   is MSRV-aware, it picks versions that work on 1.88. This one lockfile refresh
   covers everything in PR #37 and PR #32, and also brings in age 0.11.5,
   crossbeam-epoch 0.9.21, rpassword 7.5.4, and drops the yanked spin.
3. **Raise the `age` requirement floor to `0.11.5`** in `Cargo.toml`, so the
   armor and truncation fixes are enforced by the manifest and not only by the
   lockfile. That matters for Nix / `cargo install` builds that re-resolve.
4. **Stop Dependabot from bumping `dtolnay/rust-toolchain`.** Add an `ignore` entry
   in `.github/dependabot.yml`. The MSRV is changed by hand, in step with
   `rust-toolchain.toml`.

### Decisions and rejected options

- **Raise the MSRV rather than pin `rpassword = 7.4.0`.** Pinning a transitive dep
  back would fight Dependabot every week and hold back a crate that sits on the
  passphrase-input path. Nothing in the project needs 1.86. It is a binary crate,
  and the Docker test image is already on 1.96. 1.88 is the smallest bump that
  compiles, which keeps the "MSRV = oldest toolchain we test" rule honest.
- **Leave `age` 0.12 out of this change (PR #33).** 0.12 is a breaking API release
  (non-exhaustive error enums, removed `MissingPlugin` variants, plugin constructors
  now return `ResolveError`). Its security fixes are already in 0.11.5. What 0.12 adds
  on top is clearing the *unmaintained* (not vulnerable) `proc-macro-error2` warning,
  which is a build-time-only proc macro. Do it as a separate follow-up so this change
  stays small and reviewable.
- **Don't add `proc-macro-error2` to `.cargo/audit.toml`.** `cargo audit` only warns
  on it and does not fail, so an ignore entry would only hide the reminder to do the
  age 0.12 follow-up.
- **Merge PR #35 (Docker `rust:1.98-bookworm`) as-is once `main` is green.** It only
  touches `Dockerfile.test` and is independent of this change.

## Files touched

- `Cargo.toml`: `rust-version`, MSRV comment, `age` floor
- `Cargo.lock`: `cargo update`
- `rust-toolchain.toml`: channel and comment
- `.github/workflows/{ci,audit,release}.yml`: `dtolnay/rust-toolchain@1.88`
- `.github/dependabot.yml`: ignore `dtolnay/rust-toolchain`

## Deploy / blast radius

- Push the branch and open a PR; CI must be fully green.
- After merging: close #32 and #37 (superseded) and #36 (bogus), rebase #35 and
  merge it, and keep #33 open for the follow-up.
- Users building from source now need Rust ≥ 1.88. Release binaries are unaffected.

## Verification

1. `cargo audit` reports 0 vulnerabilities (the only warning left is proc-macro-error2).
2. `cargo fmt --all -- --check` passes on 1.88.
3. `cargo clippy --all-targets --all-features -- -D warnings` passes on 1.88.
4. `cargo test --all --no-fail-fast` passes (77 unit + 33 e2e).
5. PR CI is green on all jobs, including the Windows and macOS test matrix.
6. After merging, the scheduled `Audit` workflow on `main` goes green.
