use std::{
    fs::{self, File},
    io::{self, Read, Write},
    path::{Path, PathBuf},
};

use anyhow::{Context as _, Result, anyhow, bail};

use crate::{
    config::{AgeIdentities, AgeIdentity, AppConfig, Container, GitConfig},
    git,
};

pub(crate) trait Context {
    type Repo: git::Repository;

    fn repo(&self) -> &Self::Repo;

    fn store_sidecar(&self, for_path: &Path, extension: &str, content: &[u8]) -> Result<()>;

    fn load_sidecar(&self, for_path: &Path, extension: &str) -> Result<Option<Vec<u8>>>;

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

    fn get_sidecar(&self, path: &Path, extension: &str) -> Result<PathBuf> {
        let relpath = path.strip_prefix(self.repo.workdir())?;
        // The sidecar filename has to round-trip back to this path on the
        // next clean/smudge invocation, so a lossy UTF-8 conversion would
        // silently collide entries (every non-UTF8 byte folds to U+FFFD).
        // Reject explicitly instead.
        let name = relpath
            .to_str()
            .ok_or_else(|| {
                anyhow!(
                    "Path {} is not valid UTF-8; git-agecrypt requires UTF-8 paths",
                    relpath.display()
                )
            })?
            .replace('/', "!");

        let dir = self.sidecar_directory();
        fs::create_dir_all(&dir)?;

        let mut rv = dir.join(name);
        rv.set_extension(extension);
        Ok(rv)
    }
}

impl<R: git::Repository> Context for ContextWrapper<R> {
    type Repo = R;
    fn repo(&self) -> &R {
        &self.repo
    }

    fn store_sidecar(&self, for_path: &Path, extension: &str, content: &[u8]) -> Result<()> {
        let sidecar_path = self.get_sidecar(for_path, extension)?;
        File::create(sidecar_path)?.write_all(content)?;
        Ok(())
    }

    fn load_sidecar(&self, for_path: &Path, extension: &str) -> Result<Option<Vec<u8>>> {
        let sidecar_path = self.get_sidecar(for_path, extension)?;
        match File::open(sidecar_path) {
            Ok(mut f) => {
                let mut buff = Vec::new();
                f.read_to_end(&mut buff)?;
                Ok(Some(buff))
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Err(e) => {
                bail!(e)
            }
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

    /// Minimal in-memory `Repository` so the sidecar tests can pin down
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

    #[test]
    fn sidecar_path_replaces_separators() {
        let (dir, ctx) = fake_ctx();
        let nested = dir.path().join("a/b/c.txt");
        let p = ctx.get_sidecar(&nested, "age").unwrap();
        // `/` (or platform separator) must be folded to `!` so each file
        // gets a flat sidecar entry.
        let name = p.file_name().unwrap().to_string_lossy().into_owned();
        assert!(name.contains('!'), "expected '!' in sidecar name: {name}");
        assert!(name.ends_with(".age"));
    }

    #[test]
    fn sidecar_round_trips_through_store_load() {
        let (dir, ctx) = fake_ctx();
        let target = dir.path().join("secrets/note.txt");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        ctx.store_sidecar(&target, "age", b"ciphertext").unwrap();
        let loaded = ctx.load_sidecar(&target, "age").unwrap();
        assert_eq!(loaded.as_deref(), Some(&b"ciphertext"[..]));
    }

    #[test]
    fn load_sidecar_returns_none_when_missing() {
        let (dir, ctx) = fake_ctx();
        let target = dir.path().join("never-stored.txt");
        let loaded = ctx.load_sidecar(&target, "age").unwrap();
        assert!(loaded.is_none());
    }

    #[test]
    fn remove_sidecar_files_is_idempotent_when_absent() {
        let (_dir, ctx) = fake_ctx();
        // No sidecar dir exists yet — must not error.
        ctx.remove_sidecar_files().unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn sidecar_rejects_non_utf8_relpath() {
        // On Unix paths can hold arbitrary bytes; lossy conversion would
        // collide entries by folding to U+FFFD.
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let (dir, ctx) = fake_ctx();
        let bytes: &[u8] = b"weird-\xff\xfe.txt";
        let mut path = dir.path().to_path_buf();
        path.push(OsStr::from_bytes(bytes));

        let err = ctx
            .get_sidecar(&path, "age")
            .expect_err("non-UTF8 relpath must error");
        let msg = format!("{err:?}");
        assert!(
            msg.contains("not valid UTF-8"),
            "error must mention UTF-8: {msg}"
        );
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
