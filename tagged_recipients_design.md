# Native tagged recipients (`age1tag1…` / `age1tagpq1…`) — design (2026-09-30)

## Goal

Close [#17](https://github.com/bartei/git-agecrypt/issues/17): let users add the
standardized tagged recipients that recent `age-plugin-tpm` and `age-plugin-se`
emit for hardware-backed keys. As part of this, move to `age` 0.12, which also closes
Dependabot PR #33.

## Background (facts this design rests on)

- Tagged recipients are defined in the
  [C2SP age spec](https://github.com/C2SP/C2SP/blob/main/age.md#the-tagged-recipient-types)
  and have been supported in Go `age` since v1.3.0. There are two types:
  `age1tag1…` (P-256, stanza `p256tag`) and `age1tagpq1…` (hybrid ML-KEM-768 + P-256,
  stanza `mlkem768p256tag`).
- `age` **0.12.0** (crates.io, 2026-07-13) added `age::tag::Recipient` and
  `age::tagpq::Recipient`. They are always compiled in, with no feature flag.
  Both are **encryption-only**. Decrypting is done by the user's plugin identity
  (e.g. `AGE-PLUGIN-TPM-1…`), which `read_identities` already loads as a plugin
  identity. The decrypt path does not change.
- `age::plugin::Recipient`'s parser also accepts `age1tag1…`, reading the HRP
  `age1tag` as a plugin named `tag`. So the native parsers have to be tried
  **before** the plugin parser. This is the bug behind #17. #22 worked around it
  with a guard that fails with a clear error; this change replaces that guard.
- `tagpq` recipients carry the `postquantum` label. `age`'s `Encryptor` refuses to
  encrypt to a label set that differs between recipients
  (`EncryptError::IncompatibleRecipients`). So `age1tagpq1…` **cannot be combined
  with classic recipients** (x25519, ssh, `age1tag1…`) for the same file. Plugin
  recipients report their labels only when encrypting, so their compatibility
  can't be known up front.
- `age` 0.12's other breaking changes (non-exhaustive error enums, removed
  `MissingPlugin` variants, `ResolveError` from plugin constructors,
  `Send + Sync` identities) need **no source changes** here. `src/age.rs` already
  has catch-all match arms, and `?` converts `ResolveError` into anyhow errors.
- `age` 0.12 swaps the unmaintained `proc-macro-error2` for `proc-macro-error3`, so
  `cargo audit` ends up fully clean. That was the last item left from
  `deps_security_design.md`.

## Approach

1. Bump `age` to `0.12.1` in `Cargo.toml` (`cargo update -p age`). MSRV stays at 1.88.
2. In `load_public_keys` (`src/age.rs`), parse `age::tag::Recipient` and
   `age::tagpq::Recipient` after x25519/ssh and before plugin, and remove the #22
   guard.
3. Add a static recipient-mix check (`check_recipient_mix` in `src/age.rs`) that
   classifies natively known recipients as post-quantum (`tagpq`) or classic
   (x25519 / ssh / `tag`) and fails with a clear, actionable message when both
   appear. Plugin recipients are skipped because their labels can't be known
   statically; `age`'s own check still catches them when encrypting.
4. Call the mix check:
   - inside `load_public_keys`, so every encryption and validation gets it, and
   - in `AppConfig::add`, on each path's **merged** recipient list *before*
     mutating config. Mixing can build up over several `config add` calls, and
     this fails at `config add` rather than later at `git add`.
5. Update the README recipient list and replace the "not yet supported" note with a
   note on the PQ-mixing rule.

### Decisions and rejected options

- **Delegate to `age::cli_common::read_recipients`: rejected.** It would handle new
  recipient types automatically, but it also accepts recipients-file paths and
  `-i` identity paths, and its errors are CLI-oriented. Our explicit parser chain
  is small, and it's the documented place to add types.
- **Do the mix check with a trial encryption: rejected.** That would spawn plugin
  binaries during `config add`, which may prompt or need hardware. The static check
  covers every native type, and `age` backstops plugins when encrypting.
- **Also check overlapping glob patterns at `config add`: out of scope.** A file
  that matches several patterns gets the union of their recipients. A mix that only
  arises through that union still fails with `age`'s own error at `git add`. That's
  acceptable, and a full overlap analysis isn't worth the complexity here.
- **Keep the `age1tag` regression test.** The test that previously asserted a
  rejection now asserts a native `p256tag` stanza, so the plugin-misparse bug
  can't come back silently.

## Files touched

- `Cargo.toml`, `Cargo.lock`: `age` 0.12.1
- `src/age.rs`: tag/tagpq parsing, `check_recipient_mix`, tests
- `src/config/app.rs`: mix check on merged entries in `add`, test
- `README.md`: recipient types, PQ note
- `deps_security_todo.md`: tick the phase 5 and 6 items this closes

## Deploy / blast radius

- Push the branch and open a PR. After merging, close #33 (superseded) and #17.
- Behaviour change: `age1tag1…` / `age1tagpq1…` recipients are now accepted and no
  longer rejected. No existing config can break, because those recipients could not
  be added before.
- Not covered by CI: decrypting with a real TPM or Secure Enclave. Ask the #17
  reporter to confirm a round trip.

## Verification

1. A unit test shows `age1tag1…` encrypting natively (a `p256tag` stanza, no
   `age-plugin-tag` spawn).
2. A unit test shows `age1tagpq1…` encrypting natively (a `mlkem768p256tag` stanza).
3. A unit test shows `age1tagpq1…` + x25519 rejected with the mix message.
4. A unit test shows `AppConfig::add` rejecting a PQ recipient added to a path that
   already has a classic one, without changing the config.
5. `cargo fmt --check`, `cargo clippy -D warnings` and `cargo test --all` pass on 1.88.
6. `cargo audit` reports no vulnerabilities and no warnings.
7. PR CI is green on all jobs.
8. The #17 reporter confirms a TPM round trip (encrypt via git-agecrypt, decrypt via
   the plugin identity).
