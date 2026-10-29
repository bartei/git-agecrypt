// SPDX-FileCopyrightText: 2021 László Vaskó <opensource@vlaci.email>
// SPDX-License-Identifier: MPL-2.0

use std::{
    cell::OnceCell,
    fs,
    io::{self, Read, Write},
    path::Path,
    process::{Command, Stdio},
};

use ::age::secrecy::{ExposeSecret, SecretString};
use anyhow::{Context as _, Result, bail};

use crate::{
    age,
    cache::{self, CacheEntry},
    ctx::Context,
    git::Error as GitError,
    git::Repository,
};

type Identities = Vec<Box<dyn ::age::Identity>>;

pub(crate) struct CommandContext<C: Context> {
    pub ctx: C,
    /// Loaded once per process, so a long-running filter reads the secret
    /// store once. A failure is kept too: a cancelled secret-store prompt must
    /// not come back once per file.
    identities: OnceCell<std::result::Result<Identities, String>>,
}

impl<C: Context> CommandContext<C> {
    pub(crate) fn new(ctx: C) -> Self {
        Self {
            ctx,
            identities: OnceCell::new(),
        }
    }

    fn identities(&self) -> Result<&Identities> {
        self.identities
            .get_or_init(|| self.load_identities().map_err(|e| format!("{e:#}")))
            .as_ref()
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    fn load_identities(&self) -> Result<Identities> {
        let mut ids = age::load_identities(&self.get_identities()?)?;
        for command in self.ctx.identity_commands()? {
            let printed = run_identity_command(&command)?;
            ids.extend(age::identities_from_text(printed.expose_secret())?);
        }
        Ok(ids)
    }

    pub(crate) fn clean(&self, file: impl AsRef<Path>) -> Result<()> {
        let mut contents = vec![];
        io::stdin().read_to_end(&mut contents)?;
        let result = self.clean_bytes(file.as_ref(), contents)?;
        Ok(io::stdout().write_all(&result)?)
    }

    /// Encrypt for the index. Reuses the cached or HEAD ciphertext only while
    /// both the plaintext and the recipients are unchanged (vlaci/git-agecrypt
    /// 0.6 logic), so a recipient change re-encrypts instead of silently
    /// keeping the old recipients.
    pub(crate) fn clean_bytes(&self, file: &Path, contents: Vec<u8>) -> Result<Vec<u8>> {
        log::info!("Encrypting file");
        let workdir = self.ctx.repo().workdir().to_path_buf();
        let file = workdir.join(file);
        let hash = *blake3::hash(&contents).as_bytes();
        let recipients = self.ctx.config()?.get_public_keys(&file)?;
        let fp = cache::recipients_fingerprint(&recipients);
        let known =
            |e: &CacheEntry| e.recipients == fp || e.recipients == cache::UNKNOWN_RECIPIENTS;
        let entry = self.ctx.load_cache(&file)?;

        if let Some(e) = &entry
            && e.plaintext_hash == hash
            && known(e)
        {
            if e.recipients == cache::UNKNOWN_RECIPIENTS {
                self.store(&file, hash, fp, e.ciphertext.clone())?;
            }
            return Ok(e.ciphertext.clone());
        }
        if entry.as_ref().is_none_or(known)
            && let Some(head) = self.head_ciphertext_for(&file, &contents)?
        {
            self.store(&file, hash, fp, head.clone())?;
            return Ok(head);
        }
        log::debug!("File or recipients changed since last encryption, re-encrypting");
        let res = age::encrypt(&recipients, &mut &contents[..])?;
        self.store(&file, hash, fp, res.clone())?;
        Ok(res)
    }

    fn store(&self, file: &Path, hash: [u8; 32], fp: [u8; 32], ciphertext: Vec<u8>) -> Result<()> {
        self.ctx.store_cache(
            file,
            &CacheEntry {
                plaintext_hash: hash,
                recipients: fp,
                ciphertext,
            },
        )
    }

    /// HEAD's ciphertext when it decrypts to `contents`, so the index does not churn.
    fn head_ciphertext_for(&self, file: &Path, contents: &[u8]) -> Result<Option<Vec<u8>>> {
        let head = match self.ctx.repo().get_file_contents(file) {
            Ok(v) => v,
            Err(GitError::NotExist(s)) => {
                log::debug!("{s}");
                return Ok(None);
            }
            Err(e) => return Err(e.into()),
        };
        let decrypted = age::decrypt_with(self.identities()?, &head)?.unwrap_or_default();
        Ok((decrypted == contents).then_some(head))
    }

    fn get_identities(&self) -> Result<Vec<String>> {
        log::debug!("Loading identities from config");
        // Go through the AgeIdentities container so the (anchored, namespaced)
        // git-config lookup is shared with the rest of the CLI. The previous
        // direct call to `list_config("identity")` matched any config entry
        // whose name contained "identity" as a substring.
        let all_identities: Vec<String> = self
            .ctx
            .age_identities()
            .list()?
            .into_iter()
            .map(|i| i.path)
            .collect();
        log::debug!("Loaded identities from config; identities='{all_identities:?}'");
        Ok(all_identities)
    }

    pub(crate) fn smudge(&self, file: impl AsRef<Path>) -> Result<()> {
        let mut encrypted = vec![];
        io::stdin().read_to_end(&mut encrypted)?;
        let result = self.smudge_bytes(file.as_ref(), encrypted)?;
        Ok(io::stdout().write_all(&result)?)
    }

    pub(crate) fn smudge_bytes(&self, file: &Path, encrypted: Vec<u8>) -> Result<Vec<u8>> {
        log::info!("Decrypting file");
        let file = self.ctx.repo().workdir().join(file);

        if age::crlf_rewritten(&encrypted) {
            bail!(
                "{} looks like an age file whose line endings were rewritten (CRLF), so it \
                 cannot be decrypted: git treated the ciphertext as text. Add `-text` to the \
                 path's .gitattributes entry (e.g. `secrets/* filter=git-agecrypt \
                 diff=git-agecrypt -text`) and check the file out again.",
                file.display()
            );
        }
        let Some(rv) = age::decrypt_with(self.identities()?, &encrypted)? else {
            bail!("Input isn't encrypted")
        };
        log::info!("Decrypted file");
        // Unknown when the toml has no entry for this path: clean upgrades it.
        let fp = self
            .ctx
            .config()
            .ok()
            .and_then(|c| c.get_public_keys(&file).ok())
            .map(|keys| cache::recipients_fingerprint(&keys))
            .unwrap_or(cache::UNKNOWN_RECIPIENTS);
        self.store(&file, *blake3::hash(&rv).as_bytes(), fp, encrypted)?;
        Ok(rv)
    }

    /// One process per blob (git has no textconv process mode), so the
    /// identityCommand runs per blob here. Never set diff.*.cachetextconv: its
    /// notes would store plaintext in shared refs.
    pub(crate) fn textconv(&self, path: impl AsRef<Path>) -> Result<()> {
        log::info!("Decrypting file to show in diff");
        let data = fs::read(path)?;
        let result = match age::decrypt_with(self.identities()?, &data)? {
            Some(rv) => rv,
            None => {
                log::info!("File isn't encrypted, probably a working copy; showing as is.");
                data
            }
        };
        Ok(io::stdout().write_all(&result)?)
    }
}

/// The shell git itself runs filter commands through: `sh -c` on unix and
/// `cmd /C` on Windows, where no `sh` can be assumed on PATH.
fn shell_command(command: &str) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let mut c = Command::new("cmd");
        c.arg("/C").raw_arg(command);
        c
    }
    #[cfg(not(windows))]
    {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    }
}

/// Run an `identityCommand` and return what it printed. stderr stays attached
/// so a secret store can still talk to the user (unlock prompts). The output
/// is key material: it is zeroed when dropped.
fn run_identity_command(command: &str) -> Result<SecretString> {
    let out = shell_command(command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .output()
        .with_context(|| format!("cannot run identityCommand {command:?}"))?;
    if !out.status.success() {
        bail!("identityCommand {command:?} failed ({})", out.status);
    }
    let text = String::from_utf8(out.stdout).context("identityCommand output is not UTF-8")?;
    Ok(SecretString::from(text))
}

/// In-memory `Context` and `Repository` for the filter tests here and in
/// `process.rs`. Only what `clean_bytes` / `smudge_bytes` reach is
/// implemented; the rest panics so a new dependency on the repo shows up.
#[cfg(test)]
pub(crate) mod testing {
    use super::*;
    use crate::config::{AgeIdentity, AppConfig, Container};
    use crate::ctx::Context as CtxTrait;
    use crate::git;
    use ::age::secrecy::ExposeSecret;
    use std::cell::RefCell;
    use std::collections::{HashMap, HashSet};
    use std::path::PathBuf;

    pub(crate) struct TestRepo {
        pub workdir: PathBuf,
        pub gitdir: PathBuf,
        pub head: RefCell<HashMap<PathBuf, Vec<u8>>>,
        /// Paths whose HEAD lookup fails like an unreadable object store.
        pub broken: RefCell<HashSet<PathBuf>>,
    }

    impl git::Repository for TestRepo {
        fn workdir(&self) -> &Path {
            &self.workdir
        }
        fn path(&self) -> &Path {
            &self.gitdir
        }
        fn get_file_contents(&self, path: &Path) -> git::Result<Vec<u8>> {
            let rel = path.strip_prefix(&self.workdir).unwrap_or(path);
            if self.broken.borrow().contains(rel) {
                return Err(git::Error::Other(anyhow::anyhow!(
                    "object store unreadable"
                )));
            }
            self.head
                .borrow()
                .get(rel)
                .cloned()
                .ok_or_else(|| git::Error::NotExist(rel.display().to_string()))
        }
        fn add_config(&self, _: &str, _: &str) -> git::Result<()> {
            unimplemented!()
        }
        fn contains_config(&self, _: &str, _: &str) -> bool {
            unimplemented!()
        }
        fn remove_config(&self, _: &str, _: &str) -> git::Result<()> {
            unimplemented!()
        }
        fn list_config(&self, _: &str) -> git::Result<Vec<String>> {
            unimplemented!()
        }
        fn set_config(&self, _: &str, _: &str) -> git::Result<()> {
            unimplemented!()
        }
        fn remove_config_section(&self, _: &str) -> git::Result<()> {
            unimplemented!()
        }
    }

    pub(crate) struct StaticIdentities {
        pub paths: Vec<String>,
    }

    impl Container for StaticIdentities {
        type Item = AgeIdentity;
        fn add(&mut self, _: Self::Item) -> crate::config::Result<()> {
            unimplemented!()
        }
        fn remove(&mut self, _: Self::Item) -> crate::config::Result<()> {
            unimplemented!()
        }
        fn list(&self) -> crate::config::Result<Vec<Self::Item>> {
            Ok(self
                .paths
                .iter()
                .cloned()
                .map(|p| AgeIdentity { path: p })
                .collect())
        }
    }

    pub(crate) struct TestContext {
        pub repo: TestRepo,
        pub cache: RefCell<HashMap<PathBuf, CacheEntry>>,
        pub identities: Vec<String>,
        pub identity_commands: Vec<String>,
        pub config_path: PathBuf,
    }

    impl CtxTrait for TestContext {
        type Repo = TestRepo;
        fn repo(&self) -> &Self::Repo {
            &self.repo
        }

        fn store_cache(&self, path: &Path, entry: &CacheEntry) -> Result<()> {
            self.cache
                .borrow_mut()
                .insert(path.to_path_buf(), entry.clone());
            Ok(())
        }

        fn load_cache(&self, path: &Path) -> Result<Option<CacheEntry>> {
            Ok(self.cache.borrow().get(path).cloned())
        }

        fn current_exe(&self) -> Result<String> {
            unimplemented!()
        }
        fn remove_sidecar_files(&self) -> Result<()> {
            unimplemented!()
        }

        fn age_identities(&self) -> Box<dyn Container<Item = AgeIdentity> + '_> {
            Box::new(StaticIdentities {
                paths: self.identities.clone(),
            })
        }

        fn identity_commands(&self) -> Result<Vec<String>> {
            Ok(self.identity_commands.clone())
        }

        fn config(&self) -> Result<AppConfig> {
            Ok(AppConfig::load(&self.config_path, &self.repo.workdir)?)
        }
    }

    /// The path the fixture's toml grants to its own key.
    pub(crate) const FOO: &str = "secrets/foo";

    pub(crate) fn keypair() -> (::age::x25519::Identity, String, String) {
        let id = ::age::x25519::Identity::generate();
        let public = id.to_public().to_string();
        let secret = id.to_string().expose_secret().to_string();
        (id, public, secret)
    }

    /// A command for the platform shell that prints `path`.
    pub(crate) fn print_file_command(path: &Path) -> String {
        if cfg!(windows) {
            format!("type \"{}\"", path.display())
        } else {
            format!("cat \"{}\"", path.display())
        }
    }

    pub(crate) struct Fixture {
        pub dir: assert_fs::TempDir,
        pub cmd: CommandContext<TestContext>,
        pub public_key: String,
        pub identity_path: PathBuf,
    }

    impl Fixture {
        pub(crate) fn new() -> Self {
            let dir = assert_fs::TempDir::new().unwrap();
            let workdir = dir.path().to_path_buf();
            let gitdir = workdir.join(".git");
            std::fs::create_dir_all(&gitdir).unwrap();
            std::fs::create_dir_all(workdir.join("secrets")).unwrap();

            let (_id, public_key, secret) = keypair();
            let identity_path = workdir.join("test.key");
            std::fs::write(&identity_path, &secret).unwrap();

            // Recipients for secrets/foo come from the committed toml.
            let config_path = workdir.join("git-agecrypt.toml");
            std::fs::write(workdir.join(FOO), "").unwrap();
            let toml = format!("[config]\n\"{FOO}\" = [\"{public_key}\"]\n");
            std::fs::write(&config_path, toml).unwrap();

            let ctx = TestContext {
                repo: TestRepo {
                    workdir: workdir.clone(),
                    gitdir,
                    head: RefCell::new(HashMap::new()),
                    broken: RefCell::new(HashSet::new()),
                },
                cache: RefCell::new(HashMap::new()),
                identities: vec![identity_path.to_string_lossy().into_owned()],
                identity_commands: vec![],
                config_path,
            };
            Self {
                dir,
                cmd: CommandContext::new(ctx),
                public_key,
                identity_path,
            }
        }

        pub(crate) fn target(&self, rel: &str) -> PathBuf {
            self.cmd.ctx.repo.workdir.join(rel)
        }

        pub(crate) fn cache_entry(&self, plain: &[u8], recipients: [u8; 32], ciphertext: &[u8]) {
            self.cmd.ctx.cache.borrow_mut().insert(
                self.target(FOO),
                CacheEntry {
                    plaintext_hash: *blake3::hash(plain).as_bytes(),
                    recipients,
                    ciphertext: ciphertext.to_vec(),
                },
            );
        }

        pub(crate) fn cached(&self, rel: &str) -> Option<CacheEntry> {
            self.cmd.ctx.cache.borrow().get(&self.target(rel)).cloned()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn clean_reuses_cache_while_plaintext_and_recipients_match() {
        // The hot path that keeps `git status` cheap on unchanged files.
        let fx = Fixture::new();
        fx.cache_entry(
            b"hello",
            cache::recipients_fingerprint(std::slice::from_ref(&fx.public_key)),
            b"CACHED",
        );
        let out = fx
            .cmd
            .clean_bytes(Path::new(FOO), b"hello".to_vec())
            .unwrap();
        assert_eq!(out, b"CACHED");
    }

    #[test]
    fn clean_upgrades_an_entry_with_unknown_recipients() {
        // smudge stores UNKNOWN_RECIPIENTS when the toml has no entry yet;
        // the next clean serves it and records the real fingerprint.
        let fx = Fixture::new();
        fx.cache_entry(b"hello", cache::UNKNOWN_RECIPIENTS, b"CACHED");
        let out = fx
            .cmd
            .clean_bytes(Path::new(FOO), b"hello".to_vec())
            .unwrap();
        assert_eq!(out, b"CACHED");
        let fp = cache::recipients_fingerprint(std::slice::from_ref(&fx.public_key));
        assert_eq!(fx.cached(FOO).unwrap().recipients, fp);
    }

    #[test]
    fn clean_re_encrypts_when_recipients_changed() {
        // Same plaintext, cached under another recipient set: reusing it would
        // keep encrypting to a removed key.
        let fx = Fixture::new();
        fx.cache_entry(b"hello", [9; 32], b"OLD-RECIPIENTS");
        let out = fx
            .cmd
            .clean_bytes(Path::new(FOO), b"hello".to_vec())
            .unwrap();
        assert_ne!(out, b"OLD-RECIPIENTS");
        assert!(out.starts_with(b"age-encryption.org/v1"));
    }

    #[test]
    fn clean_encrypts_and_caches_without_head_or_cache() {
        let fx = Fixture::new();
        let out = fx
            .cmd
            .clean_bytes(Path::new(FOO), b"plain".to_vec())
            .unwrap();
        assert!(out.starts_with(b"age-encryption.org/v1"));
        let cached = fx.cached(FOO).unwrap();
        assert_eq!(cached.ciphertext, out);
        assert_eq!(
            cached.recipients,
            cache::recipients_fingerprint(std::slice::from_ref(&fx.public_key))
        );
    }

    #[test]
    fn clean_reuses_head_when_it_decrypts_to_the_same_plaintext() {
        // Fresh checkout, no cache: the index must not churn.
        let fx = Fixture::new();
        let head_ct =
            age::encrypt(std::slice::from_ref(&fx.public_key), &mut &b"stable"[..]).unwrap();
        fx.cmd
            .ctx
            .repo
            .head
            .borrow_mut()
            .insert(PathBuf::from(FOO), head_ct.clone());
        let out = fx
            .cmd
            .clean_bytes(Path::new(FOO), b"stable".to_vec())
            .unwrap();
        assert_eq!(out, head_ct);
    }

    #[test]
    fn clean_re_encrypts_when_head_plaintext_differs() {
        let fx = Fixture::new();
        let head_ct = age::encrypt(std::slice::from_ref(&fx.public_key), &mut &b"old"[..]).unwrap();
        fx.cmd
            .ctx
            .repo
            .head
            .borrow_mut()
            .insert(PathBuf::from(FOO), head_ct.clone());
        let out = fx.cmd.clean_bytes(Path::new(FOO), b"new".to_vec()).unwrap();
        assert_ne!(out, head_ct);
        assert!(out.starts_with(b"age-encryption.org/v1"));
    }

    #[test]
    fn clean_propagates_head_read_errors() {
        // Anything but "not in HEAD" must surface, not silently re-encrypt.
        let fx = Fixture::new();
        fx.cmd
            .ctx
            .repo
            .broken
            .borrow_mut()
            .insert(PathBuf::from(FOO));
        let err = fx
            .cmd
            .clean_bytes(Path::new(FOO), b"x".to_vec())
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("object store unreadable"),
            "{err:#}"
        );
    }

    #[test]
    fn clean_fails_for_a_path_without_recipients() {
        let fx = Fixture::new();
        let err = fx
            .cmd
            .clean_bytes(Path::new("secrets/unlisted"), b"x".to_vec())
            .unwrap_err();
        assert!(format!("{err:#}").contains("No public key"), "{err:#}");
    }

    #[test]
    fn smudge_caches_with_the_current_recipients() {
        let fx = Fixture::new();
        let ct = age::encrypt(std::slice::from_ref(&fx.public_key), &mut &b"secret"[..]).unwrap();
        let out = fx.cmd.smudge_bytes(Path::new(FOO), ct).unwrap();
        assert_eq!(out, b"secret");
        assert_eq!(
            fx.cached(FOO).unwrap().recipients,
            cache::recipients_fingerprint(std::slice::from_ref(&fx.public_key))
        );
    }

    #[test]
    fn smudge_marks_recipients_unknown_when_the_toml_has_no_entry() {
        // A checkout before the toml lists the path: cache it anyway, and let
        // the next clean fill in the fingerprint.
        let fx = Fixture::new();
        let ct = age::encrypt(std::slice::from_ref(&fx.public_key), &mut &b"secret"[..]).unwrap();
        let out = fx
            .cmd
            .smudge_bytes(Path::new("secrets/unlisted"), ct)
            .unwrap();
        assert_eq!(out, b"secret");
        assert_eq!(
            fx.cached("secrets/unlisted").unwrap().recipients,
            cache::UNKNOWN_RECIPIENTS
        );
    }

    #[test]
    fn smudge_names_the_fix_for_crlf_rewritten_ciphertext() {
        let fx = Fixture::new();
        let err = fx
            .cmd
            .smudge_bytes(
                Path::new(FOO),
                b"age-encryption.org/v1\r\n-> X25519 a\r\n".to_vec(),
            )
            .unwrap_err();
        assert!(format!("{err:#}").contains("-text"), "{err:#}");
    }

    #[test]
    fn smudge_rejects_plaintext() {
        let fx = Fixture::new();
        let err = fx
            .cmd
            .smudge_bytes(Path::new(FOO), b"not encrypted".to_vec())
            .unwrap_err();
        assert!(format!("{err:#}").contains("isn't encrypted"), "{err:#}");
    }

    #[test]
    fn textconv_decrypts_ciphertext_and_passes_plaintext_through() {
        let fx = Fixture::new();
        let ct = age::encrypt(std::slice::from_ref(&fx.public_key), &mut &b"secret"[..]).unwrap();
        let enc = fx.dir.path().join("enc.age");
        std::fs::write(&enc, ct).unwrap();
        fx.cmd.textconv(&enc).unwrap();
        // A working-copy file is plaintext already: shown as is, no error.
        let plain = fx.dir.path().join("plain.txt");
        std::fs::write(&plain, "plain").unwrap();
        fx.cmd.textconv(&plain).unwrap();
        assert!(fx.cmd.textconv(fx.dir.path().join("missing")).is_err());
    }

    #[test]
    fn get_identities_returns_configured_paths() {
        let fx = Fixture::new();
        let ids = fx.cmd.get_identities().unwrap();
        assert_eq!(ids.len(), 1);
        assert!(ids[0].ends_with("test.key"));
    }

    #[test]
    fn identity_command_supplies_the_identities() {
        // No identity file configured: the key comes from the command only.
        let mut fx = Fixture::new();
        fx.cmd.ctx.identities = vec![];
        fx.cmd.ctx.identity_commands = vec![print_file_command(&fx.identity_path)];
        let ct = age::encrypt(std::slice::from_ref(&fx.public_key), &mut &b"secret"[..]).unwrap();
        assert_eq!(fx.cmd.smudge_bytes(Path::new(FOO), ct).unwrap(), b"secret");
    }

    #[test]
    fn identity_command_failure_is_reported_once_per_process() {
        let mut fx = Fixture::new();
        fx.cmd.ctx.identities = vec![];
        fx.cmd.ctx.identity_commands = vec!["exit 1".to_owned()];
        let ct = age::encrypt(std::slice::from_ref(&fx.public_key), &mut &b"secret"[..]).unwrap();
        let first = fx.cmd.smudge_bytes(Path::new(FOO), ct.clone()).unwrap_err();
        assert!(
            format!("{first:#}").contains("identityCommand"),
            "{first:#}"
        );
        // The cached failure comes back identical instead of re-running.
        let second = fx.cmd.smudge_bytes(Path::new(FOO), ct).unwrap_err();
        assert_eq!(format!("{first:#}"), format!("{second:#}"));
    }

    #[test]
    fn identity_command_output_must_be_an_identity_file() {
        let mut fx = Fixture::new();
        fx.cmd.ctx.identities = vec![];
        fx.cmd.ctx.identity_commands = vec!["echo not-a-key".to_owned()];
        let ct = age::encrypt(std::slice::from_ref(&fx.public_key), &mut &b"secret"[..]).unwrap();
        let err = fx.cmd.smudge_bytes(Path::new(FOO), ct).unwrap_err();
        assert!(format!("{err:#}").contains("identityCommand"), "{err:#}");
    }

    #[test]
    fn run_identity_command_captures_stdout() {
        let out = run_identity_command("echo hi").unwrap();
        assert_eq!(out.expose_secret().trim(), "hi");
    }

    #[test]
    fn run_identity_command_reports_a_failing_command() {
        let err = run_identity_command("exit 3").unwrap_err();
        assert!(format!("{err:#}").contains("failed"), "{err:#}");
    }

    #[cfg(unix)]
    #[test]
    fn run_identity_command_rejects_non_utf8_output() {
        let err = run_identity_command("printf '\\377'").unwrap_err();
        assert!(format!("{err:#}").contains("not UTF-8"), "{err:#}");
    }
}
