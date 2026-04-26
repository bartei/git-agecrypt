use std::{
    collections::HashMap,
    fs, io,
    path::{Path, PathBuf},
};

use anyhow::{Context, anyhow};
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
        let invalid_paths: Vec<String> = paths
            .iter()
            .filter(|&p| !p.is_file())
            .map(|f| f.to_string_lossy().to_string())
            .collect();
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

    pub fn get_public_keys(&self, path: &Path) -> Result<&[String]> {
        let pubk = self
            .config
            .get(path.strip_prefix(&self.prefix).with_context(|| {
                format!(
                    "Not a path inside git repository, path={path:?}, repo={:?}",
                    self.prefix
                )
            })?)
            .with_context(|| format!("No public key can be found for '{}'", path.display()))?;
        Ok(&pubk[..])
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
        assert_eq!(resolved, &[pk]);
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
}
