// SPDX-FileCopyrightText: 2021 László Vaskó <opensource@vlaci.email>
// SPDX-License-Identifier: MPL-2.0

use std::{
    fs::File,
    io::{self, Read, Seek, Write},
    path::Path,
};

use anyhow::{Result, bail};

use crate::{
    age,
    cache::{self, CacheEntry},
    ctx::Context,
    git::Error as GitError,
    git::Repository,
};

pub(crate) struct CommandContext<C: Context> {
    pub ctx: C,
}

impl<C: Context> CommandContext<C> {
    pub(crate) fn new(ctx: C) -> Self {
        Self { ctx }
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
        let decrypted = age::decrypt(&self.get_identities()?, &mut &head[..])?.unwrap_or_default();
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

        let Some(rv) = age::decrypt(&self.get_identities()?, &mut &encrypted[..])? else {
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

    pub(crate) fn textconv(&self, path: impl AsRef<Path>) -> Result<()> {
        log::info!("Decrypting file to show in diff");

        let all_identities: Vec<String> = self
            .ctx
            .age_identities()
            .list()?
            .into_iter()
            .map(|i| i.path)
            .collect();

        let mut f = File::open(path)?;
        let result = if let Some(rv) = age::decrypt(&all_identities, &mut f)? {
            log::info!("Decrypted file to show in diff");
            rv
        } else {
            log::info!("File isn't encrypted, probably a working copy; showing as is.");
            f.rewind()?;
            let mut buff = vec![];
            f.read_to_end(&mut buff)?;
            buff
        };
        Ok(io::stdout().write_all(&result)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{AgeIdentity, AppConfig, Container};
    use crate::ctx::Context as CtxTrait;
    use crate::git;
    use ::age::secrecy::ExposeSecret;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::path::PathBuf;

    /// In-memory `Repository` whose only meaningful behaviour is exposing
    /// a configurable HEAD blob for a path. The other methods are not
    /// reached by `clean_bytes` so they panic if called — this catches
    /// regressions where the production code starts depending on them.
    struct TestRepo {
        workdir: PathBuf,
        gitdir: PathBuf,
        head: RefCell<HashMap<PathBuf, Vec<u8>>>,
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

    struct StaticIdentities {
        paths: Vec<String>,
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

    struct TestContext {
        repo: TestRepo,
        cache: RefCell<HashMap<PathBuf, CacheEntry>>,
        identities: Vec<String>,
        config_path: PathBuf,
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

        fn config(&self) -> Result<AppConfig> {
            Ok(AppConfig::load(&self.config_path, &self.repo.workdir)?)
        }
    }

    const FOO: &str = "secrets/foo";

    fn keypair() -> (::age::x25519::Identity, String, String) {
        let id = ::age::x25519::Identity::generate();
        let public = id.to_public().to_string();
        let secret = id.to_string().expose_secret().to_string();
        (id, public, secret)
    }

    fn build_fixture() -> (assert_fs::TempDir, CommandContext<TestContext>, String) {
        let dir = assert_fs::TempDir::new().unwrap();
        let workdir = dir.path().to_path_buf();
        let gitdir = workdir.join(".git");
        std::fs::create_dir_all(&gitdir).unwrap();
        std::fs::create_dir_all(workdir.join("secrets")).unwrap();

        let (_id, public_key, secret) = keypair();
        let id_path = workdir.join("test.key");
        std::fs::write(&id_path, &secret).unwrap();

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
            },
            cache: RefCell::new(HashMap::new()),
            identities: vec![id_path.to_string_lossy().into_owned()],
            config_path,
        };
        (dir, CommandContext::new(ctx), public_key)
    }

    fn cache_entry(
        cmd: &CommandContext<TestContext>,
        plain: &[u8],
        recipients: [u8; 32],
        ciphertext: &[u8],
    ) {
        let target = cmd.ctx.repo.workdir.join(FOO);
        cmd.ctx.cache.borrow_mut().insert(
            target,
            CacheEntry {
                plaintext_hash: *blake3::hash(plain).as_bytes(),
                recipients,
                ciphertext: ciphertext.to_vec(),
            },
        );
    }

    #[test]
    fn clean_reuses_cache_while_plaintext_and_recipients_match() {
        // The hot path that keeps `git status` cheap on unchanged files.
        let (_dir, cmd, pk) = build_fixture();
        cache_entry(
            &cmd,
            b"hello",
            cache::recipients_fingerprint(&[pk]),
            b"CACHED",
        );
        let out = cmd.clean_bytes(Path::new(FOO), b"hello".to_vec()).unwrap();
        assert_eq!(out, b"CACHED");
    }

    #[test]
    fn clean_re_encrypts_when_recipients_changed() {
        // Same plaintext, cached under another recipient set: reusing it would
        // keep encrypting to a removed key.
        let (_dir, cmd, _pk) = build_fixture();
        cache_entry(&cmd, b"hello", [9; 32], b"OLD-RECIPIENTS");
        let out = cmd.clean_bytes(Path::new(FOO), b"hello".to_vec()).unwrap();
        assert_ne!(out, b"OLD-RECIPIENTS");
        assert!(out.starts_with(b"age-encryption.org/v1"));
    }

    #[test]
    fn clean_encrypts_and_caches_without_head_or_cache() {
        let (_dir, cmd, pk) = build_fixture();
        let out = cmd.clean_bytes(Path::new(FOO), b"plain".to_vec()).unwrap();
        assert!(out.starts_with(b"age-encryption.org/v1"));
        let target = cmd.ctx.repo.workdir.join(FOO);
        let cached = cmd.ctx.cache.borrow().get(&target).cloned().unwrap();
        assert_eq!(cached.ciphertext, out);
        assert_eq!(cached.recipients, cache::recipients_fingerprint(&[pk]));
    }

    #[test]
    fn clean_reuses_head_when_it_decrypts_to_the_same_plaintext() {
        // Fresh checkout, no cache: the index must not churn.
        let (_dir, cmd, pk) = build_fixture();
        let head_ct = age::encrypt(&[pk], &mut &b"stable"[..]).unwrap();
        cmd.ctx
            .repo
            .head
            .borrow_mut()
            .insert(PathBuf::from(FOO), head_ct.clone());
        let out = cmd.clean_bytes(Path::new(FOO), b"stable".to_vec()).unwrap();
        assert_eq!(out, head_ct);
    }

    #[test]
    fn clean_re_encrypts_when_head_plaintext_differs() {
        let (_dir, cmd, pk) = build_fixture();
        let head_ct = age::encrypt(&[pk], &mut &b"old"[..]).unwrap();
        cmd.ctx
            .repo
            .head
            .borrow_mut()
            .insert(PathBuf::from(FOO), head_ct.clone());
        let out = cmd.clean_bytes(Path::new(FOO), b"new".to_vec()).unwrap();
        assert_ne!(out, head_ct);
        assert!(out.starts_with(b"age-encryption.org/v1"));
    }

    #[test]
    fn smudge_caches_with_the_current_recipients() {
        let (_dir, cmd, pk) = build_fixture();
        let ct = age::encrypt(std::slice::from_ref(&pk), &mut &b"secret"[..]).unwrap();
        let out = cmd.smudge_bytes(Path::new(FOO), ct).unwrap();
        assert_eq!(out, b"secret");
        let target = cmd.ctx.repo.workdir.join(FOO);
        let cached = cmd.ctx.cache.borrow().get(&target).cloned().unwrap();
        assert_eq!(cached.recipients, cache::recipients_fingerprint(&[pk]));
    }

    #[test]
    fn get_identities_returns_configured_paths() {
        let (_dir, cmd, _pk) = build_fixture();
        let ids = cmd.get_identities().unwrap();
        assert_eq!(ids.len(), 1);
        assert!(ids[0].ends_with("test.key"));
    }
}
