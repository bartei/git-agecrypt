// SPDX-FileCopyrightText: 2021 László Vaskó <opensource@vlaci.email>
// SPDX-License-Identifier: MPL-2.0

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result, bail};

use crate::{
    cache::{self, CacheEntry},
    config::{AgeIdentities, AgeIdentity, AppConfig, Container, GitConfig},
    git,
};

pub(crate) trait Context {
    type Repo: git::Repository;

    fn repo(&self) -> &Self::Repo;

    fn store_cache(&self, for_path: &Path, entry: &CacheEntry) -> Result<()>;

    fn load_cache(&self, for_path: &Path) -> Result<Option<CacheEntry>>;

    fn current_exe(&self) -> Result<String>;

    fn remove_sidecar_files(&self) -> Result<()>;

    fn age_identities(&self) -> Box<dyn Container<Item = AgeIdentity> + '_>;

    fn config(&self) -> Result<AppConfig>;
}

struct ContextWrapper<R: git::Repository> {
    repo: R,
}

impl<R: git::Repository> ContextWrapper<R> {
    pub(crate) fn new(repo: R) -> Self {
        Self { repo }
    }

    fn sidecar_directory(&self) -> PathBuf {
        self.repo.path().join("git-agecrypt")
    }

    // Cache layout, entry format and atomic writes come from vlaci/git-agecrypt
    // 0.6 (László Vaskó, MPL-2.0). Entries are keyed by a hash of the path:
    // bartei's `/` to `!` plus set_extension naming collided for `a.pdf` and
    // `a.json`, and for paths containing `!`.
    fn cache_directory(&self) -> PathBuf {
        self.sidecar_directory().join("v2")
    }

    fn cache_path(&self, for_path: &Path) -> Result<PathBuf> {
        let relpath = for_path.strip_prefix(self.repo.workdir())?;
        Ok(self.cache_directory().join(cache::file_name(relpath)))
    }
}

impl<R: git::Repository> Context for ContextWrapper<R> {
    type Repo = R;
    fn repo(&self) -> &R {
        &self.repo
    }

    fn store_cache(&self, for_path: &Path, entry: &CacheEntry) -> Result<()> {
        let final_path = self.cache_path(for_path)?;
        let dir = self.cache_directory();
        let mut builder = fs::DirBuilder::new();
        builder.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder.create(&dir)?;

        let name = final_path.file_name().context("cache entry has no name")?;
        let tmp = dir.join(format!(
            "{}.tmp{}",
            name.to_string_lossy(),
            std::process::id()
        ));
        let write_result = (|| -> io::Result<()> {
            let mut opts = OpenOptions::new();
            opts.write(true).create(true).truncate(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                opts.mode(0o600);
            }
            opts.open(&tmp)?.write_all(&entry.encode())?;
            fs::rename(&tmp, &final_path)
        })();
        if let Err(e) = write_result {
            let _ = fs::remove_file(&tmp);
            return Err(e.into());
        }
        Ok(())
    }

    fn load_cache(&self, for_path: &Path) -> Result<Option<CacheEntry>> {
        let path = self.cache_path(for_path)?;
        match File::open(&path) {
            Ok(mut f) => {
                let mut buff = Vec::new();
                f.read_to_end(&mut buff)?;
                let entry = CacheEntry::decode(&buff);
                if entry.is_none() {
                    log::warn!("Ignoring unreadable cache entry {}", path.display());
                }
                Ok(entry)
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => bail!(e),
        }
    }

    fn current_exe(&self) -> Result<String> {
        let exe = std::env::current_exe()?;
        // The exe path is embedded into `.git/config` as the filter / diff
        // driver command. git's config parser interprets backslashes as
        // escape introducers (`\n`, `\t`, …) and silently swallows unknown
        // escapes — `D:\a\git-agecrypt\…\git-agecrypt.exe` round-trips as
        // `D:agit-agecrypttargetdebuggit-agecrypt.exe` and the spawn fails
        // with "command not found". Normalise to forward slashes; both
        // `git` and `cmd.exe` accept forward-slash paths on Windows.
        let s = exe.to_str().with_context(|| {
            format!(
                "Executable path {} is not valid UTF-8; git filter commands cannot encode it",
                exe.display()
            )
        })?;
        let normalized = if cfg!(windows) {
            s.replace('\\', "/")
        } else {
            s.to_string()
        };
        Ok(normalized)
    }

    fn remove_sidecar_files(&self) -> Result<()> {
        let dir = self.sidecar_directory();
        fs::remove_dir_all(dir).or_else(|err| {
            if err.kind() == std::io::ErrorKind::NotFound {
                Ok(())
            } else {
                Err(err)
            }
        })?;
        Ok(())
    }

    fn age_identities(&self) -> Box<dyn Container<Item = AgeIdentity> + '_> {
        let cfg = GitConfig::new(self, "identity".into());
        Box::new(AgeIdentities::new(cfg))
    }

    fn config(&self) -> Result<AppConfig> {
        Ok(AppConfig::load(
            &PathBuf::from("git-agecrypt.toml"),
            self.repo.workdir(),
        )?)
    }
}

pub(crate) fn new(repo: git::LibGit2Repository) -> impl Context<Repo = git::LibGit2Repository> {
    ContextWrapper::new(repo)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal in-memory `Repository` so the cache tests can pin down
    /// `workdir`/`path` without spinning up libgit2.
    struct FakeRepo {
        workdir: PathBuf,
        gitdir: PathBuf,
    }

    impl git::Repository for FakeRepo {
        fn workdir(&self) -> &Path {
            &self.workdir
        }
        fn path(&self) -> &Path {
            &self.gitdir
        }
        fn get_file_contents(&self, _: &Path) -> git::Result<Vec<u8>> {
            unimplemented!()
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

    fn fake_ctx() -> (assert_fs::TempDir, ContextWrapper<FakeRepo>) {
        let dir = assert_fs::TempDir::new().unwrap();
        let workdir = dir.path().to_path_buf();
        let gitdir = workdir.join(".git");
        std::fs::create_dir_all(&gitdir).unwrap();
        let ctx = ContextWrapper::new(FakeRepo { workdir, gitdir });
        (dir, ctx)
    }

    fn entry(ciphertext: &[u8]) -> CacheEntry {
        CacheEntry {
            plaintext_hash: [1; 32],
            recipients: [2; 32],
            ciphertext: ciphertext.to_vec(),
        }
    }

    #[test]
    fn cache_names_are_flat_and_distinct_per_path() {
        let (dir, ctx) = fake_ctx();
        let name = |rel: &str| ctx.cache_path(&dir.path().join(rel)).unwrap();
        let pdf = name("ring/E-2026-0042.pdf");
        assert_eq!(pdf.parent().unwrap(), ctx.cache_directory());
        // Same stem, other extension: bartei's naming mapped both to one entry.
        assert_ne!(pdf, name("ring/E-2026-0042.json"));
        // `!` in a path: the old `/` to `!` folding mapped both to one entry.
        assert_ne!(name("a!b/c"), name("a/b!c"));
    }

    #[test]
    fn cache_round_trips_through_store_load() {
        let (dir, ctx) = fake_ctx();
        let target = dir.path().join("secrets/note.txt");
        ctx.store_cache(&target, &entry(b"ciphertext")).unwrap();
        let loaded = ctx.load_cache(&target).unwrap().unwrap();
        assert_eq!(loaded.ciphertext, b"ciphertext");
        assert_eq!(loaded.recipients, [2; 32]);
    }

    #[cfg(unix)]
    #[test]
    fn cache_files_are_private() {
        use std::os::unix::fs::PermissionsExt;
        let (dir, ctx) = fake_ctx();
        let target = dir.path().join("secrets/note.txt");
        ctx.store_cache(&target, &entry(b"x")).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&ctx.cache_path(&target).unwrap()), 0o600);
        assert_eq!(mode(&ctx.cache_directory()), 0o700);
    }

    #[test]
    fn load_cache_returns_none_when_missing() {
        let (dir, ctx) = fake_ctx();
        let target = dir.path().join("never-stored.txt");
        assert!(ctx.load_cache(&target).unwrap().is_none());
    }

    #[test]
    fn remove_sidecar_files_is_idempotent_when_absent() {
        let (_dir, ctx) = fake_ctx();
        // No cache dir exists yet — must not error.
        ctx.remove_sidecar_files().unwrap();
    }

    #[test]
    fn current_exe_returns_normalized_string() {
        // Smoke test: in the test process the current_exe is the test
        // binary path and is always valid UTF-8.
        let (_dir, ctx) = fake_ctx();
        let s = ctx.current_exe().unwrap();
        if cfg!(windows) {
            assert!(
                !s.contains('\\'),
                "windows must normalise to forward slashes"
            );
        }
        assert!(!s.is_empty());
    }
}
