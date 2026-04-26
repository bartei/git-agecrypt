//! End-to-end integration tests for the git-agecrypt binary.
//!
//! Each test creates an ephemeral git repository in a temp directory,
//! drives the real `git-agecrypt` binary through its full CLI surface,
//! and asserts on real filesystem / git index state. Together with the
//! unit tests in `src/git.rs`, these cover every public command.

use std::fs;
use std::path::{Path, PathBuf};

use age::secrecy::ExposeSecret;
use assert_fs::TempDir;
use duct::cmd;

fn agecrypt_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_git-agecrypt"))
}

struct Fixture {
    dir: TempDir,
    public_key: String,
    identity_path: PathBuf,
}

impl Fixture {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        cmd!("git", "init", "--initial-branch=main")
            .dir(dir.path())
            .stdout_null()
            .stderr_null()
            .run()
            .unwrap();
        cmd!("git", "config", "user.email", "test@example.com")
            .dir(dir.path())
            .run()
            .unwrap();
        cmd!("git", "config", "user.name", "Test")
            .dir(dir.path())
            .run()
            .unwrap();
        cmd!("git", "config", "commit.gpgsign", "false")
            .dir(dir.path())
            .run()
            .unwrap();

        let identity = age::x25519::Identity::generate();
        let public_key = identity.to_public().to_string();
        let identity_path = dir.path().join("test.key");
        fs::write(&identity_path, identity.to_string().expose_secret()).unwrap();

        Self {
            dir,
            public_key,
            identity_path,
        }
    }

    fn workdir(&self) -> &Path {
        self.dir.path()
    }

    fn run(&self, args: &[&str]) -> std::process::Output {
        cmd(agecrypt_bin(), args)
            .dir(self.workdir())
            .stdout_capture()
            .stderr_capture()
            .unchecked()
            .run()
            .unwrap()
    }

    fn run_ok(&self, args: &[&str]) -> String {
        let out = self.run(args);
        assert!(
            out.status.success(),
            "agecrypt {:?} failed: stdout={} stderr={}",
            args,
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr),
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn write(&self, rel: &str, contents: &str) {
        let p = self.workdir().join(rel);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(p, contents).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.workdir().join(rel)).unwrap()
    }

    fn git(&self, args: &[&str]) -> String {
        let out = cmd("git", args)
            .dir(self.workdir())
            .stdout_capture()
            .stderr_capture()
            .unchecked()
            .run()
            .unwrap();
        assert!(
            out.status.success(),
            "git {:?} failed: stderr={}",
            args,
            String::from_utf8_lossy(&out.stderr),
        );
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    fn git_config_get_all(&self, key: &str) -> Vec<String> {
        let out = cmd("git", &["config", "--get-all", key])
            .dir(self.workdir())
            .stdout_capture()
            .stderr_capture()
            .unchecked()
            .run()
            .unwrap();
        if !out.status.success() {
            return vec![];
        }
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .map(String::from)
            .collect()
    }

    fn install_filter(&self) {
        // Wire git-agecrypt as the filter/diff driver, then mark the path
        // as encryptable via .gitattributes.
        self.run_ok(&["init"]);
        fs::write(
            self.workdir().join(".gitattributes"),
            "secrets/* filter=git-agecrypt diff=git-agecrypt\n",
        )
        .unwrap();
    }

    fn add_recipient_for(&self, rel_path: &str) {
        // The CLI validates that the path exists; create an empty placeholder
        // if the test hasn't written content yet.
        let full = format!("secrets/{rel_path}");
        let p = self.workdir().join(&full);
        if let Some(parent) = p.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        if !p.exists() {
            fs::write(&p, "").unwrap();
        }
        self.run_ok(&["config", "add", "-r", &self.public_key, "-p", &full]);
    }

    fn add_identity(&self) {
        self.run_ok(&["config", "add", "-i", self.identity_path.to_str().unwrap()]);
    }

    /// The identity path in the form it appears in stored / listed CLI
    /// output. `AgeIdentity::try_from` normalises Windows backslashes to
    /// forward slashes before storing in git config, so test assertions
    /// must compare against the same canonical form.
    fn identity_path_in_config(&self) -> String {
        let s = self.identity_path.to_str().unwrap();
        if cfg!(windows) {
            s.replace('\\', "/")
        } else {
            s.to_string()
        }
    }
}

// ----- init / deinit -----

#[test]
fn init_writes_filter_and_diff_config() {
    let fx = Fixture::new();
    fx.run_ok(&["init"]);

    assert_eq!(
        fx.git_config_get_all("filter.git-agecrypt.required"),
        vec!["true".to_string()]
    );
    let smudge = fx.git_config_get_all("filter.git-agecrypt.smudge");
    assert!(smudge.len() == 1 && smudge[0].contains("smudge -f %f"));
    let clean = fx.git_config_get_all("filter.git-agecrypt.clean");
    assert!(clean.len() == 1 && clean[0].contains("clean -f %f"));
    let textconv = fx.git_config_get_all("diff.git-agecrypt.textconv");
    assert!(textconv.len() == 1 && textconv[0].contains("textconv"));
}

#[test]
fn init_quotes_exe_path_in_filter_commands() {
    // Paths with spaces (or other shell metacharacters) must round-trip
    // through `sh -c` correctly. Verify that the stored filter command
    // is a quoted form, so installs into e.g. `~/Application Support/`
    // keep working.
    let fx = Fixture::new();
    fx.run_ok(&["init"]);
    for key in [
        "filter.git-agecrypt.smudge",
        "filter.git-agecrypt.clean",
        "diff.git-agecrypt.textconv",
    ] {
        let value = fx.git_config_get_all(key);
        assert_eq!(value.len(), 1, "missing config for {key}");
        assert!(
            value[0].starts_with('"'),
            "filter command for {key} must start with a double-quote so paths with spaces are shell-safe; got: {}",
            value[0]
        );
        // The quoted form must close the quote before the subcommand,
        // e.g. `"…/git-agecrypt" smudge -f %f`.
        assert!(
            value[0].matches('"').count() >= 2,
            "filter command for {key} must contain a balanced pair of quotes; got: {}",
            value[0]
        );
    }
}

#[test]
fn init_is_idempotent() {
    let fx = Fixture::new();
    fx.run_ok(&["init"]);
    fx.run_ok(&["init"]);
    let smudge = fx.git_config_get_all("filter.git-agecrypt.smudge");
    assert_eq!(smudge.len(), 1, "init must not duplicate config entries");
}

#[test]
fn deinit_removes_both_filter_and_diff_sections() {
    // Regression test for the historical typo "fiter.git-agecrypt"
    // that left the filter section behind on deinit.
    let fx = Fixture::new();
    fx.run_ok(&["init"]);
    fx.run_ok(&["deinit"]);

    assert!(
        fx.git_config_get_all("filter.git-agecrypt.smudge")
            .is_empty()
    );
    assert!(
        fx.git_config_get_all("filter.git-agecrypt.clean")
            .is_empty()
    );
    assert!(
        fx.git_config_get_all("filter.git-agecrypt.required")
            .is_empty()
    );
    assert!(
        fx.git_config_get_all("diff.git-agecrypt.textconv")
            .is_empty()
    );
}

#[test]
fn deinit_removes_sidecar_directory() {
    let fx = Fixture::new();
    fx.install_filter();
    fx.add_recipient_for("a");
    fx.add_identity();
    fx.write("secrets/a", "hello sidecar");
    fx.git(&["add", "secrets/a"]);
    let sidecar_dir = fx.workdir().join(".git").join("git-agecrypt");
    assert!(sidecar_dir.exists());

    fx.run_ok(&["deinit"]);
    assert!(!sidecar_dir.exists());
}

// ----- config: identity -----

#[test]
fn config_add_list_remove_identity() {
    let fx = Fixture::new();
    fx.add_identity();

    let listed = fx.run_ok(&["config", "list", "-i"]);
    let needle = fx.identity_path_in_config();
    assert!(
        listed.contains(&needle),
        "expected listed output to contain {needle:?}, got:\n{listed}"
    );
    assert!(listed.contains("✓"), "valid identity should be marked ✓");

    fx.run_ok(&["config", "remove", "-i", fx.identity_path.to_str().unwrap()]);
    let after = fx.run_ok(&["config", "list", "-i"]);
    assert!(!after.contains(&needle));
}

#[test]
fn config_add_invalid_identity_fails() {
    let fx = Fixture::new();
    let bogus = fx.workdir().join("not-a-key");
    fs::write(&bogus, "this is not an age identity").unwrap();
    let out = fx.run(&["config", "add", "-i", bogus.to_str().unwrap()]);
    assert!(!out.status.success(), "invalid identity must be rejected");
}

// ----- config: recipient -----

#[test]
fn config_add_list_remove_recipient() {
    let fx = Fixture::new();
    fs::create_dir_all(fx.workdir().join("secrets")).unwrap();
    fx.write("secrets/a", "");
    fx.write("secrets/b", "");

    fx.run_ok(&[
        "config",
        "add",
        "-r",
        &fx.public_key,
        "-p",
        "secrets/a",
        "secrets/b",
    ]);
    let listed = fx.run_ok(&["config", "list", "-r"]);
    assert!(listed.contains("secrets/a"));
    assert!(listed.contains("secrets/b"));
    assert!(listed.contains(&fx.public_key));

    // Removing one path should leave the other.
    fx.run_ok(&["config", "remove", "-r", &fx.public_key, "-p", "secrets/a"]);
    let after = fx.run_ok(&["config", "list", "-r"]);
    assert!(!after.contains("secrets/a"));
    assert!(after.contains("secrets/b"));
}

#[test]
fn config_add_recipient_dedup() {
    // Phase 1 fix: AppConfig::add must dedup non-consecutive duplicates too.
    let fx = Fixture::new();
    fs::create_dir_all(fx.workdir().join("secrets")).unwrap();
    fx.write("secrets/a", "");
    fx.run_ok(&["config", "add", "-r", &fx.public_key, "-p", "secrets/a"]);
    fx.run_ok(&["config", "add", "-r", &fx.public_key, "-p", "secrets/a"]);

    let toml = fx.read("git-agecrypt.toml");
    let occurrences = toml.matches(&fx.public_key).count();
    assert_eq!(
        occurrences, 1,
        "recipient must not be duplicated, got toml:\n{toml}"
    );
}

#[test]
fn config_add_invalid_recipient_fails() {
    let fx = Fixture::new();
    fs::create_dir_all(fx.workdir().join("secrets")).unwrap();
    fx.write("secrets/a", "");
    let out = fx.run(&["config", "add", "-r", "not-a-public-key", "-p", "secrets/a"]);
    assert!(!out.status.success());
}

#[test]
fn config_add_recipient_for_missing_path_fails() {
    let fx = Fixture::new();
    let out = fx.run(&[
        "config",
        "add",
        "-r",
        &fx.public_key,
        "-p",
        "secrets/does-not-exist",
    ]);
    assert!(!out.status.success());
}

// ----- status -----

#[test]
fn status_reports_configured_identities_and_recipients() {
    let fx = Fixture::new();
    fx.install_filter();
    fx.add_identity();
    fx.add_recipient_for("a");
    fx.write("secrets/a", "");

    let out = fx.run_ok(&["status"]);
    assert!(out.contains(&fx.identity_path_in_config()));
    assert!(out.contains(&fx.public_key));
}

// ----- the encryption pipeline (clean / smudge / textconv) -----

#[test]
fn clean_smudge_round_trip_via_git() {
    let fx = Fixture::new();
    fx.install_filter();
    fx.add_identity();
    fx.write("secrets/secret.txt", "hello world");
    fx.add_recipient_for("secret.txt");

    fx.git(&[
        "add",
        ".gitattributes",
        "git-agecrypt.toml",
        "secrets/secret.txt",
    ]);
    fx.git(&["commit", "-m", "initial"]);

    // The blob in the git index/HEAD must be encrypted.
    let head_blob = fx.git(&["show", "HEAD:secrets/secret.txt"]);
    assert!(
        head_blob.starts_with("age-encryption.org/v1") || head_blob.contains("BEGIN AGE"),
        "blob in git was not encrypted; got: {head_blob:?}"
    );
    assert!(!head_blob.contains("hello world"));

    // The working copy must still be plaintext.
    assert_eq!(fx.read("secrets/secret.txt"), "hello world");

    // Re-checkout from HEAD must decrypt back to plaintext (smudge).
    fs::remove_file(fx.workdir().join("secrets/secret.txt")).unwrap();
    fx.git(&["checkout", "--", "secrets/secret.txt"]);
    assert_eq!(fx.read("secrets/secret.txt"), "hello world");
}

#[test]
fn unchanged_file_does_not_re_encrypt() {
    // The blake3-hash sidecar must short-circuit re-encryption so the
    // ciphertext blob in git stays stable across `git add` calls.
    let fx = Fixture::new();
    fx.install_filter();
    fx.add_identity();
    fx.write("secrets/stable.txt", "stable content");
    fx.add_recipient_for("stable.txt");
    fx.git(&[
        "add",
        ".gitattributes",
        "git-agecrypt.toml",
        "secrets/stable.txt",
    ]);
    fx.git(&["commit", "-m", "first"]);

    let blob_before = fx.git(&["show", "HEAD:secrets/stable.txt"]);

    // Touch the file mtime but keep contents identical, then re-add.
    fs::write(fx.workdir().join("secrets/stable.txt"), "stable content").unwrap();
    fx.git(&["add", "secrets/stable.txt"]);

    // After a no-op re-add the ciphertext should still match exactly.
    let staged_blob = fx.git(&["show", ":secrets/stable.txt"]);
    assert_eq!(staged_blob, blob_before);
}

#[test]
fn textconv_decrypts_for_diff() {
    let fx = Fixture::new();
    fx.install_filter();
    fx.add_identity();
    fx.write("secrets/diff.txt", "line one\nline two\n");
    fx.add_recipient_for("diff.txt");
    fx.git(&[
        "add",
        ".gitattributes",
        "git-agecrypt.toml",
        "secrets/diff.txt",
    ]);
    fx.git(&["commit", "-m", "v1"]);

    fx.write("secrets/diff.txt", "line one\nline two changed\n");
    let diff = fx.git(&["diff", "secrets/diff.txt"]);
    assert!(
        diff.contains("line two") && diff.contains("line two changed"),
        "git diff did not decrypt via textconv; got:\n{diff}"
    );
}

#[test]
fn smudge_rejects_unencrypted_input() {
    // The smudge filter is only meant to be invoked by git on
    // ciphertext blobs; running it on plaintext must fail loudly
    // rather than silently emitting garbage.
    let fx = Fixture::new();
    fx.run_ok(&["init"]);
    fx.add_identity();

    let out = cmd(agecrypt_bin(), &["smudge", "-f", "anything"])
        .dir(fx.workdir())
        .stdin_bytes("definitely not age-encrypted\n".as_bytes())
        .stdout_capture()
        .stderr_capture()
        .unchecked()
        .run()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn textconv_passes_through_plaintext_working_copy() {
    // textconv is invoked by git for both committed (encrypted) blobs and
    // the working-copy (plaintext) version of a file. The plaintext path
    // must round-trip unchanged so that diff output makes sense.
    let fx = Fixture::new();
    fx.install_filter();
    fx.add_identity();
    fx.write("secrets/note.txt", "plain working copy");
    fx.add_recipient_for("note.txt");

    // Run textconv directly against the unencrypted on-disk file.
    let out = cmd(
        agecrypt_bin(),
        &[
            "textconv",
            &fx.workdir().join("secrets/note.txt").to_string_lossy(),
        ],
    )
    .dir(fx.workdir())
    .stdout_capture()
    .stderr_capture()
    .unchecked()
    .run()
    .unwrap();
    assert!(out.status.success());
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim_end_matches('\n'),
        "plain working copy"
    );
}

#[test]
fn config_remove_recipient_for_unknown_path_fails() {
    // Removing a recipient for a path that was never registered must
    // surface an error — silently no-oping would mask user typos.
    let fx = Fixture::new();
    let out = fx.run(&[
        "config",
        "remove",
        "-r",
        &fx.public_key,
        "-p",
        "secrets/never-added",
    ]);
    assert!(!out.status.success());
}

#[test]
fn config_remove_recipient_globally_clears_all_paths() {
    // `config remove -r <r>` (no -p) should drop the recipient from every
    // path it appears under and leave any other recipients in place.
    let fx = Fixture::new();
    fs::create_dir_all(fx.workdir().join("secrets")).unwrap();
    fx.write("secrets/a", "");
    fx.write("secrets/b", "");

    let other_pk = age::x25519::Identity::generate().to_public().to_string();
    fx.run_ok(&[
        "config",
        "add",
        "-r",
        &fx.public_key,
        &other_pk,
        "-p",
        "secrets/a",
        "secrets/b",
    ]);

    fx.run_ok(&["config", "remove", "-r", &fx.public_key]);

    let listed = fx.run_ok(&["config", "list", "-r"]);
    assert!(!listed.contains(&fx.public_key));
    assert!(listed.contains(&other_pk));
}

#[test]
fn config_remove_path_only_drops_path() {
    // `config remove -p <p>` (no -r) should remove the path entry entirely.
    let fx = Fixture::new();
    fs::create_dir_all(fx.workdir().join("secrets")).unwrap();
    fx.write("secrets/a", "");
    fx.write("secrets/b", "");
    fx.run_ok(&[
        "config",
        "add",
        "-r",
        &fx.public_key,
        "-p",
        "secrets/a",
        "secrets/b",
    ]);
    fx.run_ok(&["config", "remove", "-p", "secrets/a"]);

    let listed = fx.run_ok(&["config", "list", "-r"]);
    assert!(!listed.contains("secrets/a"));
    assert!(listed.contains("secrets/b"));
}

#[test]
fn deinit_without_init_is_noop() {
    // `deinit` must be idempotent in the absence of prior init — same
    // policy as `init` itself (existing-config errors are downgraded).
    let fx = Fixture::new();
    fx.run_ok(&["deinit"]);
}

#[test]
fn config_remove_identity_that_was_never_added_fails() {
    let fx = Fixture::new();
    let out = fx.run(&["config", "remove", "-i", fx.identity_path.to_str().unwrap()]);
    assert!(!out.status.success());
}

#[test]
fn config_list_recipients_when_empty_succeeds() {
    let fx = Fixture::new();
    let out = fx.run_ok(&["config", "list", "-r"]);
    assert!(out.contains("recipients"));
}

#[test]
fn config_list_identities_when_empty_succeeds() {
    let fx = Fixture::new();
    let out = fx.run_ok(&["config", "list", "-i"]);
    assert!(out.contains("identities"));
}

#[test]
fn status_on_fresh_repo_does_not_error() {
    // Status with nothing configured must still succeed and print
    // both sections (just empty).
    let fx = Fixture::new();
    fx.run_ok(&["status"]);
}

#[test]
fn invalid_subcommand_fails_with_clap_error() {
    // Belt-and-braces: a missing subcommand should fail clap parsing
    // rather than crashing the binary.
    let out = cmd(agecrypt_bin(), &["this-is-not-a-command"])
        .dir(std::env::temp_dir())
        .stdout_capture()
        .stderr_capture()
        .unchecked()
        .run()
        .unwrap();
    assert!(!out.status.success());
}

#[test]
fn config_add_invalid_listed_marker_when_identity_breaks() {
    // If a configured identity file goes missing or becomes garbage,
    // `config list -i` must continue to work and mark it with `⨯`.
    let fx = Fixture::new();
    fx.add_identity();

    // Corrupt the on-disk identity behind the configured path.
    fs::write(&fx.identity_path, "no longer a valid identity").unwrap();

    let listed = fx.run_ok(&["config", "list", "-i"]);
    assert!(
        listed.contains("⨯"),
        "broken identity should be marked invalid; got:\n{listed}"
    );
}
