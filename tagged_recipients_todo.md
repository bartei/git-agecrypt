# Native tagged recipients — todo (see tagged_recipients_design.md for detail)

## Phase 1 — Upgrade age
- [x] Bump `age` to 0.12.1
- [x] Confirm it compiles on 1.88 with no source changes

## Phase 2 — Recipient parsing
- [x] Parse `age::tag::Recipient` before the plugin parser
- [x] Parse `age::tagpq::Recipient` before the plugin parser
- [x] Remove the #22 guard
- [x] Add `check_recipient_mix` for PQ vs classic
- [x] Call the mix check from `load_public_keys`
- [x] Call the mix check on merged entries in `AppConfig::add`

## Phase 3 — Tests
- [x] `age1tag1…` encrypts to a `p256tag` stanza
- [x] `age1tagpq1…` encrypts to a `mlkem768p256tag` stanza
- [x] tagpq + x25519 rejected with a clear message
- [x] `AppConfig::add` rejects a PQ/classic mix and leaves config untouched

## Phase 4 — Docs
- [x] Update README recipient list and PQ note
- [x] Tick closed items in `deps_security_todo.md`

## Phase 5 — Verify
- [x] `cargo fmt --check`, `clippy -D warnings`, `cargo test --all` pass
- [x] `cargo audit` fully clean

## Phase 6 — Land
- [ ] Push branch and open PR, CI green
- [ ] Merge PR
- [ ] Close #33 and #17
- [ ] #17 reporter confirms a TPM round trip
