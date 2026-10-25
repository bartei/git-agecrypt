# Dependency security sweep — todo (see deps_security_design.md for detail)

## Phase 1 — Restore a buildable main
- [x] Raise MSRV to 1.88 in `Cargo.toml` and `rust-toolchain.toml`
- [x] Point all `dtolnay/rust-toolchain` refs in workflows at 1.88

## Phase 2 — Clear advisories
- [x] `cargo update` (semver-compatible refresh)
- [x] Raise `age` floor to 0.11.5 in `Cargo.toml`
- [x] `cargo audit` shows 0 vulnerabilities

## Phase 3 — Verify locally
- [x] `cargo fmt --check` passes
- [x] `cargo clippy -D warnings` passes
- [x] `cargo test --all` passes

## Phase 4 — Dependabot hygiene
- [x] Ignore `dtolnay/rust-toolchain` in `dependabot.yml`

## Phase 5 — Land and triage PRs
- [x] Open PR for `fix/security-deps`, CI green
- [x] Merge PR
- [x] Close #32 and #37 as superseded
- [x] Close #36 as bogus
- [ ] Rebase and merge #35
- [ ] Scheduled `Audit` on main is green

## Phase 6 — Follow-up
- [ ] Migrate to `age` 0.12 (PR #33) to drop unmaintained proc-macro-error2
