use std::{
    fs::File,
    io::{self, Read, Seek, Write},
    path::{Path, PathBuf},
};

use anyhow::{Result, bail};
use blake3::Hash;

use crate::{age, ctx::Context, git::Error as GitError, git::Repository};

pub(crate) struct CommandContext<C: Context> {
    pub ctx: C,
}

impl<C: Context> CommandContext<C> {
    pub(crate) fn clean(&self, file: impl AsRef<Path>) -> Result<()> {
        log::info!("Encrypting file");
        let file = self.ctx.repo().workdir().join(file);

        log::debug!("Looking for saved hash information. target={file:?}",);
        let mut existing_hash = [0u8; 32];
        if let Some(hash_buffer) = self.ctx.load_sidecar(&file, "hash")? {
            existing_hash = hash_buffer.as_slice().try_into()?
        } else {
            log::debug!("No saved hash file found");
        }

        let mut hasher = blake3::Hasher::new();
        let mut contents = vec![];
        io::stdin().read_to_end(&mut contents)?;
        let hash = hasher.update(&contents).finalize();

        let old_hash = Hash::from(existing_hash);
        log::debug!(
            "Comparing hashes for file; old_hash={}, new_hash={:?}",
            old_hash.to_hex().as_str(),
            hash.to_hex().as_str()
        );

        let saved = if hash == old_hash {
            self.ctx.load_sidecar(&file, "age")?
        } else {
            None
        };

        let result = self.get_content(contents, hash, file, saved)?;
        Ok(io::stdout().write_all(&result)?)
    }

    fn get_content(
        &self,
        contents: Vec<u8>,
        hash: Hash,
        file: PathBuf,
        saved_content: Option<Vec<u8>>,
    ) -> Result<Vec<u8>> {
        if let Some(saved_content) = saved_content {
            log::debug!("File didn't change since last encryption, loading from git HEAD");
            return Ok(saved_content);
        }

        log::debug!("Encrypted content changed, checking decrypted version");
        let repo_contents = match self.ctx.repo().get_file_contents(&file) {
            Ok(v) => Some(v),
            Err(GitError::NotExist(s)) => {
                log::debug!("{s}");
                None
            }
            Err(e) => return Err(e.into()),
        };

        if let Some(repo_contents) = repo_contents {
            let identities = self.get_identities()?;
            let mut cur = io::Cursor::new(repo_contents);
            let decrypted = age::decrypt(&identities, &mut cur)?.unwrap_or_default();
            if decrypted == contents {
                log::debug!("Decrypted content matches, using from working copy");
                self.ctx.store_sidecar(&file, "hash", hash.as_bytes())?;
                self.ctx.store_sidecar(&file, "age", cur.get_ref())?;
                return Ok(cur.into_inner());
            }
        }

        log::debug!("File changed since last encryption, re-encrypting");

        let cfg = self.ctx.config()?;
        let public_keys = cfg.get_public_keys(&file)?;

        let res = age::encrypt(public_keys, &mut &contents[..])?;
        self.ctx.store_sidecar(&file, "hash", hash.as_bytes())?;
        self.ctx.store_sidecar(&file, "age", &res)?;
        Ok(res)
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
        log::info!("Decrypting file");
        let file = self.ctx.repo().workdir().join(file);

        let mut encrypted = vec![];
        io::stdin().read_to_end(&mut encrypted)?;
        let mut cur = io::Cursor::new(encrypted);
        let all_identities = self.get_identities()?;
        if let Some(rv) = age::decrypt(&all_identities, &mut cur)? {
            log::info!("Decrypted file");
            let mut hasher = blake3::Hasher::new();
            let hash = hasher.update(&rv).finalize();

            log::debug!("Storing hash for file; hash={:?}", hash.to_hex().as_str(),);
            self.ctx.store_sidecar(&file, "hash", hash.as_bytes())?;
            self.ctx.store_sidecar(&file, "age", cur.get_ref())?;

            Ok(io::stdout().write_all(&rv)?)
        } else {
            bail!("Input isn't encrypted")
        }
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

    /// In-memory `Repository` whose only meaningful behaviour is exposing
    /// a configurable HEAD blob for a path. The other methods are not
    /// reached by `get_content` so they panic if called — this catches
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
        sidecars: RefCell<HashMap<(PathBuf, String), Vec<u8>>>,
        identities: Vec<String>,
        config_path: PathBuf,
    }

    impl CtxTrait for TestContext {
        type Repo = TestRepo;
        fn repo(&self) -> &Self::Repo {
            &self.repo
        }

        fn store_sidecar(&self, path: &Path, ext: &str, content: &[u8]) -> Result<()> {
            self.sidecars
                .borrow_mut()
                .insert((path.to_path_buf(), ext.to_string()), content.to_vec());
            Ok(())
        }

        fn load_sidecar(&self, path: &Path, ext: &str) -> Result<Option<Vec<u8>>> {
            Ok(self
                .sidecars
                .borrow()
                .get(&(path.to_path_buf(), ext.to_string()))
                .cloned())
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

        // The clean filter's reencrypt branch reads recipients from the
        // committed `git-agecrypt.toml`. Build one mapping secrets/foo →
        // our test recipient.
        let config_path = workdir.join("git-agecrypt.toml");
        std::fs::write(workdir.join("secrets/foo"), "").unwrap();
        let toml = format!("[config]\n\"secrets/foo\" = [\"{public_key}\"]\n");
        std::fs::write(&config_path, toml).unwrap();

        let ctx = TestContext {
            repo: TestRepo {
                workdir: workdir.clone(),
                gitdir,
                head: RefCell::new(HashMap::new()),
            },
            sidecars: RefCell::new(HashMap::new()),
            identities: vec![id_path.to_string_lossy().into_owned()],
            config_path,
        };
        let cmd = CommandContext { ctx };
        (dir, cmd, public_key)
    }

    fn hash_of(plain: &[u8]) -> blake3::Hash {
        let mut h = blake3::Hasher::new();
        h.update(plain).finalize()
    }

    #[test]
    fn get_content_uses_saved_when_provided() {
        // saved_content == Some → fast path returns it verbatim, with no
        // encryption work or sidecar mutation. This is the hot path that
        // keeps `git status` cheap on unchanged files.
        let (_dir, cmd, _pk) = build_fixture();
        let target = cmd.ctx.repo.workdir.join("secrets/foo");
        let plaintext = b"hello".to_vec();
        let cached_ct = b"FAKE_CIPHERTEXT".to_vec();

        let out = cmd
            .get_content(
                plaintext.clone(),
                hash_of(&plaintext),
                target,
                Some(cached_ct.clone()),
            )
            .unwrap();
        assert_eq!(out, cached_ct, "saved content must be used verbatim");
        assert!(
            cmd.ctx.sidecars.borrow().is_empty(),
            "fast path must not touch sidecars"
        );
    }

    #[test]
    fn get_content_re_encrypts_when_no_head_and_no_sidecar() {
        // First-ever clean: no saved sidecar AND nothing in HEAD → must
        // produce fresh ciphertext and persist both sidecars.
        let (_dir, cmd, _pk) = build_fixture();
        let target = cmd.ctx.repo.workdir.join("secrets/foo");
        let plaintext = b"plain".to_vec();
        let hash = hash_of(&plaintext);

        let out = cmd
            .get_content(plaintext.clone(), hash, target.clone(), None)
            .unwrap();
        assert_ne!(out, plaintext, "output must be ciphertext, not plaintext");
        assert!(
            !out.is_empty(),
            "ciphertext must be non-empty for non-empty input"
        );
        let s = cmd.ctx.sidecars.borrow();
        assert!(
            s.contains_key(&(target.clone(), "hash".to_string())),
            "hash sidecar must be written"
        );
        assert!(
            s.contains_key(&(target, "age".to_string())),
            "age sidecar must be written"
        );
    }

    #[test]
    fn get_content_reuses_head_when_decrypted_matches() {
        // No saved sidecar, but HEAD already has the file encrypted to
        // the same plaintext (e.g. fresh checkout of a tracked file).
        // The clean filter must return HEAD's ciphertext verbatim so the
        // git index doesn't churn.
        let (_dir, cmd, pk) = build_fixture();
        let target = cmd.ctx.repo.workdir.join("secrets/foo");
        let plaintext = b"stable".to_vec();

        let head_ct = age::encrypt(&[pk], &mut &plaintext[..]).unwrap();
        cmd.ctx
            .repo
            .head
            .borrow_mut()
            .insert(PathBuf::from("secrets/foo"), head_ct.clone());

        let out = cmd
            .get_content(plaintext.clone(), hash_of(&plaintext), target, None)
            .unwrap();
        assert_eq!(
            out, head_ct,
            "must reuse HEAD ciphertext rather than re-encrypt"
        );
    }

    #[test]
    fn get_content_re_encrypts_when_head_plaintext_differs() {
        // HEAD has the file encrypted to a *different* plaintext
        // (the working copy edited it). Must re-encrypt the new
        // plaintext and overwrite the sidecars.
        let (_dir, cmd, pk) = build_fixture();
        let target = cmd.ctx.repo.workdir.join("secrets/foo");
        let old_plain = b"old".to_vec();
        let new_plain = b"new".to_vec();

        let head_ct = age::encrypt(&[pk], &mut &old_plain[..]).unwrap();
        cmd.ctx
            .repo
            .head
            .borrow_mut()
            .insert(PathBuf::from("secrets/foo"), head_ct.clone());

        let out = cmd
            .get_content(new_plain.clone(), hash_of(&new_plain), target, None)
            .unwrap();
        assert_ne!(out, head_ct, "must produce fresh ciphertext, not HEAD's");
        assert_ne!(out, new_plain, "output must be ciphertext, not plaintext");
    }

    #[test]
    fn get_identities_returns_configured_paths() {
        let (_dir, cmd, _pk) = build_fixture();
        let ids = cmd.get_identities().unwrap();
        assert_eq!(ids.len(), 1);
        assert!(ids[0].ends_with("test.key"));
    }
}
