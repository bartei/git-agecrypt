use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, anyhow};
use globset::Glob;
use serde::{Deserialize, Serialize};

use crate::age;

use super::Result;

#[derive(Serialize, Deserialize)]
pub struct AppConfig {
    config: HashMap<PathBuf, Vec<String>>,
    #[serde(skip)]
    path: PathBuf,
    #[serde(skip)]
    prefix: PathBuf,
}

/// A path entry in `git-agecrypt.toml` is treated as a glob pattern when it
/// contains any of the standard glob meta-characters. Patterns without these
/// characters keep behaving exactly as before (literal exact match), so
/// existing configurations don't see any change.
fn is_glob(s: &str) -> bool {
    s.contains('*') || s.contains('?') || s.contains('[')
}

impl AppConfig {
    pub fn load(path: &Path, repo_prefix: &Path) -> Result<Self> {
        match fs::read_to_string(path) {
            Ok(contents) => {
                let mut cfg: AppConfig = toml::from_str(&contents).with_context(|| {
                    format!("Couldn't load configuration file '{}'", path.display())
                })?;
                cfg.path = path.into();
                cfg.prefix = repo_prefix.into();
                Ok(cfg)
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self {
                config: HashMap::new(),
                path: path.into(),
                prefix: repo_prefix.into(),
            }),
            Err(err) => Ok(Err(err).with_context(|| {
                format!("Couldn't read configuration file '{}'", path.display())
            })?),
        }
    }

    pub fn save(&self) -> Result<()> {
        let cfg = toml::to_string_pretty(self).context("Couldn't format configuration as TOML")?;
        fs::write(&self.path, cfg).with_context(|| {
            format!("Couldn't save configuration file '{}'", self.path.display())
        })?;
        Ok(())
    }

    pub fn add(&mut self, recipients: Vec<String>, paths: Vec<PathBuf>) -> Result<()> {
        age::validate_public_keys(&recipients)?;

        // Validate that glob patterns parse, and that literal paths refer to
        // files that actually exist. Glob entries by definition don't have to
        // resolve to anything on disk -- they cover files that may show up
        // later (e.g. `**/terraform.tfstate` registered before any state file
        // has been generated).
        let mut invalid_paths: Vec<String> = Vec::new();
        let mut bad_globs: Vec<String> = Vec::new();
        for p in &paths {
            let s = p.to_string_lossy();
            if is_glob(&s) {
                if let Err(e) = Glob::new(&s) {
                    bad_globs.push(format!("{s} ({e})"));
                }
            } else if !p.is_file() {
                invalid_paths.push(s.into_owned());
            }
        }
        if !bad_globs.is_empty() {
            return Err(anyhow!(
                "The following glob patterns are invalid: {}",
                bad_globs.join(", ")
            )
            .into());
        }
        if !invalid_paths.is_empty() {
            return Err(anyhow!(
                "The following files don't exist: {}",
                invalid_paths.join(", ")
            )
            .into());
        }
        for path in paths {
            let entry = self.config.entry(path).or_default();
            for r in &recipients {
                if !entry.contains(r) {
                    entry.push(r.clone());
                }
            }
        }
        Ok(())
    }

    pub fn remove(&mut self, recipients: Vec<String>, paths: Vec<PathBuf>) -> Result<()> {
        if paths.is_empty() {
            for rs in self.config.values_mut() {
                rs.retain(|r| !recipients.contains(r));
            }
        } else {
            for path in paths {
                let rs = self.config.get_mut(&path).with_context(|| {
                    format!("No configuration entry found for {}", path.display())
                })?;
                if recipients.is_empty() {
                    rs.clear();
                } else {
                    rs.retain(|r| !recipients.contains(r));
                }
            }
        }

        self.config.retain(|_, rs| !rs.is_empty());

        Ok(())
    }

    pub fn list(&self) -> Vec<(String, String)> {
        let mut rv = vec![];
        for (p, rs) in &self.config {
            for r in rs {
                rv.push((p.to_string_lossy().to_string(), r.clone()));
            }
        }
        rv
    }

    /// Resolve the set of recipients that should be used to encrypt the file
    /// at `path`. Returns the union of recipients from every entry in
    /// `git-agecrypt.toml` whose key either matches `path` literally or
    /// matches it as a glob pattern. Order is stable across invocations: a
    /// literal exact match (if present) is returned first, followed by the
    /// globs in the order they appear in the on-disk file.
    pub fn get_public_keys(&self, path: &Path) -> Result<Vec<String>> {
        let rel = path.strip_prefix(&self.prefix).with_context(|| {
            format!(
                "Not a path inside git repository, path={path:?}, repo={:?}",
                self.prefix
            )
        })?;

        let mut collected: Vec<String> = Vec::new();
        let push_dedup = |r: &String, out: &mut Vec<String>| {
            if !out.contains(r) {
                out.push(r.clone());
            }
        };

        // 1. Exact literal match first, so the most-specific recipient set
        //    shows up at the head of the list (and we don't even compile a
        //    GlobMatcher when an exact entry exists).
        if let Some(recipients) = self.config.get(rel) {
            for r in recipients {
                push_dedup(r, &mut collected);
            }
        }

        // 2. Glob entries -- iterated in the file's serialized order so the
        //    final recipient ordering is reproducible.
        for (key, recipients) in &self.config {
            let key_str = key.to_string_lossy();
            if !is_glob(&key_str) {
                continue;
            }
            let matcher = Glob::new(&key_str)
                .with_context(|| format!("Invalid glob pattern in git-agecrypt.toml: {key_str}"))?
                .compile_matcher();
            if matcher.is_match(rel) {
                for r in recipients {
                    push_dedup(r, &mut collected);
                }
            }
        }

        if collected.is_empty() {
            return Err(anyhow!("No public key can be found for '{}'", path.display()).into());
        }
        Ok(collected)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use assert_fs::TempDir;
    use std::sync::{Mutex, MutexGuard, OnceLock};

    fn cwd_lock() -> MutexGuard<'static, ()> {
        // `AppConfig::add` resolves path arguments via `Path::is_file`, which is
        // CWD-relative. Tests must therefore run serially with a known CWD,
        // otherwise concurrent tests race on the process-global directory.
        static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
        LOCK.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }

    struct CwdGuard {
        previous: PathBuf,
        _lock: MutexGuard<'static, ()>,
    }

    impl CwdGuard {
        fn enter(dir: &Path) -> Self {
            let lock = cwd_lock();
            let previous = std::env::current_dir().unwrap();
            std::env::set_current_dir(dir).unwrap();
            Self {
                previous,
                _lock: lock,
            }
        }
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            let _ = std::env::set_current_dir(&self.previous);
        }
    }

    fn pubkey() -> String {
        ::age::x25519::Identity::generate().to_public().to_string()
    }

    fn fixture() -> (TempDir, PathBuf) {
        let dir = TempDir::new().unwrap();
        let cfg = dir.path().join("git-agecrypt.toml");
        (dir, cfg)
    }

    #[test]
    fn load_returns_empty_when_file_missing() {
        let (dir, cfg) = fixture();
        let app = AppConfig::load(&cfg, dir.path()).unwrap();
        assert!(app.list().is_empty());
    }

    #[test]
    fn load_save_round_trip_preserves_entries() {
        let (dir, cfg) = fixture();
        let pk = pubkey();
        let secret = dir.path().join("secrets/foo");
        fs::create_dir_all(secret.parent().unwrap()).unwrap();
        fs::write(&secret, "").unwrap();

        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(vec![pk.clone()], vec![PathBuf::from("secrets/foo")])
            .unwrap();
        app.save().unwrap();

        // Reload — the on-disk TOML must parse back into the same data.
        let reloaded = AppConfig::load(&cfg, dir.path()).unwrap();
        let listed = reloaded.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, "secrets/foo");
        assert_eq!(listed[0].1, pk);
    }

    #[test]
    fn load_returns_error_on_malformed_toml() {
        let (dir, cfg) = fixture();
        fs::write(&cfg, "this is not valid toml = = =").unwrap();
        assert!(AppConfig::load(&cfg, dir.path()).is_err());
    }

    #[test]
    fn add_rejects_invalid_recipient() {
        let (dir, cfg) = fixture();
        let secret = dir.path().join("secrets/foo");
        fs::create_dir_all(secret.parent().unwrap()).unwrap();
        fs::write(&secret, "").unwrap();

        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        let result = app.add(
            vec!["definitely-not-a-pubkey".to_string()],
            vec![PathBuf::from("secrets/foo")],
        );
        assert!(result.is_err());
    }

    #[test]
    fn add_rejects_missing_path() {
        let (dir, cfg) = fixture();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        let result = app.add(vec![pubkey()], vec![PathBuf::from("does/not/exist")]);
        assert!(result.is_err());
    }

    #[test]
    fn add_dedups_within_single_call() {
        let (dir, cfg) = fixture();
        let secret = dir.path().join("a");
        fs::write(&secret, "").unwrap();
        let pk = pubkey();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(vec![pk.clone(), pk.clone()], vec![PathBuf::from("a")])
            .unwrap();
        let listed = app.list();
        assert_eq!(listed.len(), 1, "duplicate recipients must be collapsed");
    }

    #[test]
    fn add_dedups_across_calls() {
        let (dir, cfg) = fixture();
        let secret = dir.path().join("a");
        fs::write(&secret, "").unwrap();
        let pk = pubkey();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(vec![pk.clone()], vec![PathBuf::from("a")]).unwrap();
        app.add(vec![pk.clone()], vec![PathBuf::from("a")]).unwrap();
        assert_eq!(app.list().len(), 1);
    }

    #[test]
    fn remove_specific_recipient_leaves_other_paths_alone() {
        let (dir, cfg) = fixture();
        let pk1 = pubkey();
        let pk2 = pubkey();
        for f in ["a", "b"] {
            fs::write(dir.path().join(f), "").unwrap();
        }
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(
            vec![pk1.clone(), pk2.clone()],
            vec![PathBuf::from("a"), PathBuf::from("b")],
        )
        .unwrap();

        // Remove pk1 globally (no path argument).
        app.remove(vec![pk1.clone()], vec![]).unwrap();
        let listed = app.list();
        assert_eq!(listed.len(), 2, "only pk2 should remain on a and b");
        assert!(listed.iter().all(|(_, r)| r == &pk2));
    }

    #[test]
    fn remove_all_recipients_for_path_drops_path() {
        let (dir, cfg) = fixture();
        let pk = pubkey();
        fs::write(dir.path().join("a"), "").unwrap();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(vec![pk.clone()], vec![PathBuf::from("a")]).unwrap();

        // Removing without recipients clears the path entirely.
        app.remove(vec![], vec![PathBuf::from("a")]).unwrap();
        assert!(app.list().is_empty());
    }

    #[test]
    fn remove_specific_recipient_for_path() {
        let (dir, cfg) = fixture();
        let pk1 = pubkey();
        let pk2 = pubkey();
        fs::write(dir.path().join("a"), "").unwrap();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(vec![pk1.clone(), pk2.clone()], vec![PathBuf::from("a")])
            .unwrap();

        app.remove(vec![pk1.clone()], vec![PathBuf::from("a")])
            .unwrap();
        let listed = app.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].1, pk2);
    }

    #[test]
    fn remove_unknown_path_errors() {
        let (dir, cfg) = fixture();
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        let result = app.remove(vec![], vec![PathBuf::from("nope")]);
        assert!(result.is_err());
    }

    #[test]
    fn get_public_keys_strips_repo_prefix() {
        let (dir, cfg) = fixture();
        let pk = pubkey();
        let abs = dir.path().join("a");
        fs::write(&abs, "").unwrap();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(vec![pk.clone()], vec![PathBuf::from("a")]).unwrap();

        let resolved = app.get_public_keys(&abs).unwrap();
        assert_eq!(resolved, vec![pk]);
    }

    #[test]
    fn get_public_keys_outside_repo_errors() {
        let (dir, cfg) = fixture();
        let app = AppConfig::load(&cfg, dir.path()).unwrap();
        let outside = TempDir::new().unwrap();
        let result = app.get_public_keys(&outside.path().join("foo"));
        assert!(result.is_err());
    }

    #[test]
    fn get_public_keys_unknown_path_errors() {
        let (dir, cfg) = fixture();
        let app = AppConfig::load(&cfg, dir.path()).unwrap();
        let result = app.get_public_keys(&dir.path().join("never-added"));
        assert!(result.is_err());
    }

    // ----- glob path matching -----

    #[test]
    fn add_accepts_glob_pattern_without_existing_file() {
        // A literal path is rejected if the file doesn't exist (regression guard),
        // but a glob is by design a forward-looking pattern -- it must be
        // registrable before any matching file exists.
        let (dir, cfg) = fixture();
        let pk = pubkey();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(
            vec![pk.clone()],
            vec![PathBuf::from("**/terraform.tfstate")],
        )
        .unwrap();
        assert_eq!(app.list().len(), 1);
    }

    #[test]
    fn add_rejects_invalid_glob_pattern() {
        let (dir, cfg) = fixture();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        // Unclosed character class is invalid glob syntax.
        let result = app.add(vec![pubkey()], vec![PathBuf::from("secrets/[unclosed")]);
        assert!(result.is_err());
    }

    #[test]
    fn get_public_keys_matches_double_star_glob_at_any_depth() {
        // The whole point of this feature: `**/file` matches `file` at any
        // depth without per-instance registration.
        let (dir, cfg) = fixture();
        let pk = pubkey();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(
            vec![pk.clone()],
            vec![PathBuf::from("**/terraform.tfstate")],
        )
        .unwrap();

        for depth in &[
            "terraform.tfstate",
            "a/terraform.tfstate",
            "a/b/c/d/terraform.tfstate",
        ] {
            let resolved = app
                .get_public_keys(&dir.path().join(depth))
                .unwrap_or_else(|e| panic!("expected glob to match {depth}: {e}"));
            assert_eq!(resolved, vec![pk.clone()], "depth {depth} should match");
        }
    }

    #[test]
    fn get_public_keys_glob_does_not_match_unrelated_files() {
        let (dir, cfg) = fixture();
        let pk = pubkey();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(vec![pk], vec![PathBuf::from("**/terraform.tfstate")])
            .unwrap();

        let result = app.get_public_keys(&dir.path().join("a/b/other-file.txt"));
        assert!(result.is_err(), "non-matching path must surface as no-key");
    }

    #[test]
    fn get_public_keys_literal_and_glob_recipients_merge() {
        // A file matched by both an exact entry and a glob entry must end up
        // encrypted to the UNION of both recipient sets. This is what makes
        // "default everyone via glob, plus targeted extras via literal" work.
        let (dir, cfg) = fixture();
        let pk_glob = pubkey();
        let pk_literal = pubkey();
        let placeholder = dir.path().join("dev/terraform.tfstate");
        fs::create_dir_all(placeholder.parent().unwrap()).unwrap();
        fs::write(&placeholder, "").unwrap();

        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(
            vec![pk_glob.clone()],
            vec![PathBuf::from("**/terraform.tfstate")],
        )
        .unwrap();
        app.add(
            vec![pk_literal.clone()],
            vec![PathBuf::from("dev/terraform.tfstate")],
        )
        .unwrap();

        let resolved = app.get_public_keys(&placeholder).unwrap();
        assert_eq!(resolved.len(), 2);
        // Literal exact match is collected first, glob entries come after.
        assert_eq!(resolved[0], pk_literal);
        assert!(resolved.contains(&pk_glob));
    }

    #[test]
    fn get_public_keys_dedupes_when_same_recipient_matches_multiple_globs() {
        // Two overlapping globs that share a recipient must not produce
        // duplicate entries in the output (would cost an extra recipient slot
        // in the age header and inflate the ciphertext for no reason).
        let (dir, cfg) = fixture();
        let pk = pubkey();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(vec![pk.clone()], vec![PathBuf::from("**/*.tfstate")])
            .unwrap();
        app.add(
            vec![pk.clone()],
            vec![PathBuf::from("dev/**/terraform.tfstate")],
        )
        .unwrap();

        let resolved = app
            .get_public_keys(&dir.path().join("dev/foo/terraform.tfstate"))
            .unwrap();
        assert_eq!(resolved, vec![pk]);
    }

    #[test]
    fn remove_with_glob_key_drops_entry() {
        // Removal stays exact-string keyed: pass the same glob you registered
        // and the entry disappears.
        let (dir, cfg) = fixture();
        let pk = pubkey();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(
            vec![pk.clone()],
            vec![PathBuf::from("**/terraform.tfstate")],
        )
        .unwrap();
        app.remove(vec![pk], vec![PathBuf::from("**/terraform.tfstate")])
            .unwrap();
        assert!(app.list().is_empty());
    }

    #[test]
    fn save_load_round_trip_preserves_glob_keys_verbatim() {
        // The on-disk TOML must keep the user's glob pattern as-is; we don't
        // want to silently rewrite/expand it.
        let (dir, cfg) = fixture();
        let pk = pubkey();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(
            vec![pk.clone()],
            vec![PathBuf::from("**/terraform.tfstate")],
        )
        .unwrap();
        app.save().unwrap();

        let reloaded = AppConfig::load(&cfg, dir.path()).unwrap();
        let listed = reloaded.list();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].0, "**/terraform.tfstate");
        assert_eq!(listed[0].1, pk);
    }
}
