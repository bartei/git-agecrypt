use std::{
    collections::{BTreeMap, HashMap},
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
    /// Names for keys, so `config` entries can list people instead of
    /// recipients: `teo = "age1…"` or `teo = ["age1…", "age1…"]`.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    keys: BTreeMap<String, Keys>,
    #[serde(skip)]
    path: PathBuf,
    #[serde(skip)]
    prefix: PathBuf,
}

#[derive(Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum Keys {
    One(String),
    Many(Vec<String>),
}

impl Keys {
    fn all(&self) -> Vec<String> {
        match self {
            Self::One(k) => vec![k.clone()],
            Self::Many(ks) => ks.clone(),
        }
    }
}

/// A `config` entry that is a recipient itself rather than a `[keys]` name.
fn is_recipient(entry: &str) -> bool {
    entry.starts_with("age1") || entry.starts_with("ssh-")
}

/// Key names are plain words, never mistaken for a recipient. No `.`: a bare
/// dotted key in TOML is a nested table, so `a.b = "…"` would not round-trip.
fn validate_key_name(name: &str) -> Result<()> {
    let plain = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-'));
    if !plain || is_recipient(name) {
        return Err(
            anyhow!("key name '{name}': letters, digits, _ and - only, not a recipient").into(),
        );
    }
    Ok(())
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
                let mut cfg = Self::parse(&contents).with_context(|| {
                    format!("Couldn't load configuration file '{}'", path.display())
                })?;
                cfg.path = path.into();
                cfg.prefix = repo_prefix.into();
                Ok(cfg)
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(Self {
                config: HashMap::new(),
                keys: BTreeMap::new(),
                path: path.into(),
                prefix: repo_prefix.into(),
            }),
            Err(err) => Ok(Err(err).with_context(|| {
                format!("Couldn't read configuration file '{}'", path.display())
            })?),
        }
    }

    /// A configuration from text, for reading a policy without a worktree.
    /// Key names are checked here: one spelled like a recipient could
    /// redirect every ring that lists that recipient.
    pub fn parse(contents: &str) -> anyhow::Result<Self> {
        let cfg: Self = toml::from_str(contents)?;
        for name in cfg.keys.keys() {
            validate_key_name(name)?;
        }
        Ok(cfg)
    }

    /// Edit the file in place so comments, layout and unrelated tables
    /// survive; only entries that changed are rewritten.
    pub fn save(&self) -> Result<()> {
        let text = match fs::read_to_string(&self.path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => String::new(),
            Err(err) => Err(err).context("Couldn't read configuration file")?,
        };
        let mut doc: toml_edit::DocumentMut =
            text.parse().context("Couldn't parse configuration file")?;
        if self.keys.is_empty() {
            doc.remove("keys");
        } else {
            let at = doc
                .get("config")
                .and_then(|c| c.as_table())
                .and_then(|t| t.position());
            let keys = edit_table(&mut doc, "keys");
            keys.retain(|name, _| self.keys.contains_key(name));
            for (name, k) in &self.keys {
                let all = k.all();
                if keys.get(name).is_some_and(|v| strings(v) == all) {
                    continue;
                }
                keys[name.as_str()] = match k {
                    Keys::One(r) => toml_edit::value(r.as_str()),
                    Keys::Many(rs) => toml_edit::value(rs.iter().collect::<toml_edit::Array>()),
                };
            }
            // [keys] reads best above the rings that use it.
            if keys.position().is_none() || at.is_some_and(|c| keys.position() > Some(c)) {
                keys.set_position(at.map(|c| c - 1).or(Some(-1)));
            }
        }
        let config = edit_table(&mut doc, "config");
        if !self.keys.is_empty()
            && config
                .decor()
                .prefix()
                .and_then(|p| p.as_str())
                .is_none_or(str::is_empty)
        {
            config.decor_mut().set_prefix("\n");
        }
        config.retain(|p, _| self.config.contains_key(Path::new(p)));
        let mut paths: Vec<&PathBuf> = self.config.keys().collect();
        paths.sort();
        // Untouched entries keep their layout (multi-line arrays, comments).
        for p in paths {
            let key = p.to_string_lossy();
            if config
                .get(&key)
                .is_some_and(|v| strings(v) == self.config[p])
            {
                continue;
            }
            config[&*key] = toml_edit::value(self.config[p].iter().collect::<toml_edit::Array>());
        }
        fs::write(&self.path, doc.to_string()).with_context(|| {
            format!("Couldn't save configuration file '{}'", self.path.display())
        })?;
        Ok(())
    }

    /// Recipients behind `config` entries: a `[keys]` name stands for its
    /// keys, a recipient for itself. An unknown name fails rather than
    /// silently encrypting to fewer people.
    pub fn resolve(&self, entries: &[String]) -> Result<Vec<String>> {
        let mut out: Vec<String> = Vec::new();
        for e in entries {
            let keys = match self.keys.get(e) {
                _ if is_recipient(e) => vec![e.clone()],
                Some(k) => k.all(),
                None => {
                    return Err(anyhow!("unknown key name '{e}': define it under [keys]").into());
                }
            };
            for k in keys {
                if !out.contains(&k) {
                    out.push(k);
                }
            }
        }
        Ok(out)
    }

    /// Name keys, then grant the name on `paths`: `teo` stands for both
    /// `age1…` in every ring that lists it.
    pub fn add_named(
        &mut self,
        name: String,
        recipients: Vec<String>,
        paths: Vec<PathBuf>,
    ) -> Result<()> {
        validate_key_name(&name)?;
        age::validate_public_keys(&recipients)?;
        let mut keys = self.keys.get(&name).map(Keys::all).unwrap_or_default();
        for r in recipients {
            if !keys.contains(&r) {
                keys.push(r);
            }
        }
        let keys = match <[String; 1]>::try_from(keys) {
            Ok([one]) => Keys::One(one),
            Err(many) => Keys::Many(many),
        };
        self.keys.insert(name.clone(), keys);
        self.add(vec![name], paths)
    }

    pub fn add(&mut self, recipients: Vec<String>, paths: Vec<PathBuf>) -> Result<()> {
        let literal: Vec<String> = recipients
            .iter()
            .filter(|r| !self.keys.contains_key(*r))
            .cloned()
            .collect();
        age::validate_public_keys(&literal)?;

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
        // Removing only the literal entries would leave the key granted
        // through its name.
        for (name, keys) in &self.keys {
            if let Some(r) = keys.all().iter().find(|k| recipients.contains(k)) {
                return Err(anyhow!(
                    "{r} is in [keys] {name}: remove -r {name}, or edit [keys] by hand"
                )
                .into());
            }
        }
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
        self.resolve(&collected)
    }
}

/// The strings of a toml value: a string or an array of them.
fn strings(item: &toml_edit::Item) -> Vec<String> {
    match item.as_value() {
        Some(toml_edit::Value::String(s)) => vec![s.value().clone()],
        Some(toml_edit::Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str().map(str::to_owned))
            .collect(),
        _ => vec![],
    }
}

/// The table `name` of `doc`, created when missing.
fn edit_table<'a>(doc: &'a mut toml_edit::DocumentMut, name: &str) -> &'a mut toml_edit::Table {
    if !doc.contains_table(name) {
        doc.insert(name, toml_edit::Item::Table(toml_edit::Table::new()));
    }
    doc[name].as_table_mut().expect("just ensured a table")
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

    #[test]
    fn remove_refuses_a_recipient_held_by_a_key_name() {
        // Dropping only the literal entry would leave the key behind its
        // name: the revoked key would keep decrypting.
        let (dir, cfg) = fixture();
        let (pk, other) = (pubkey(), pubkey());
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add_named("bob".into(), vec![pk.clone()], vec![PathBuf::from("r/**")])
            .unwrap();
        app.add(vec![other], vec![PathBuf::from("r/**")]).unwrap();
        let err = app.remove(vec![pk.clone()], vec![]).unwrap_err();
        assert!(format!("{err:#}").contains("bob"), "{err:#}");
        assert!(
            app.get_public_keys(&dir.path().join("r/x"))
                .unwrap()
                .contains(&pk)
        );
    }

    #[test]
    fn key_names_must_be_plain_words() {
        let (dir, cfg) = fixture();
        let secret = dir.path().join("secrets/foo");
        fs::create_dir_all(secret.parent().unwrap()).unwrap();
        fs::write(&secret, "").unwrap();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        for bad in [
            "",
            "age1abc",
            "ssh-ed25519",
            "two words",
            "tab\there",
            "dotted.name",
        ] {
            let err = app
                .add_named(
                    bad.into(),
                    vec![pubkey()],
                    vec![PathBuf::from("secrets/foo")],
                )
                .unwrap_err();
            assert!(format!("{err:#}").contains("key name"), "{bad:?}: {err:#}");
        }
        // The same check guards a hand-edited file.
        let err = AppConfig::parse(&format!(
            "[keys]\n\"age1abc\" = \"{}\"\n[config]\n",
            pubkey()
        ))
        .err()
        .expect("a recipient-shaped key name must be rejected");
        assert!(format!("{err:#}").contains("key name"), "{err:#}");
        assert!(AppConfig::parse("[keys]\nteo_x-1 = \"age1abc\"\n[config]\n").is_ok());
    }

    #[test]
    fn unknown_key_name_fails_resolution() {
        let (dir, cfg) = fixture();
        fs::write(&cfg, "[config]\n\"secrets/foo\" = [\"nobody\"]\n").unwrap();
        let app = AppConfig::load(&cfg, dir.path()).unwrap();
        let err = app
            .get_public_keys(&dir.path().join("secrets/foo"))
            .unwrap_err();
        assert!(
            format!("{err:#}").contains("unknown key name 'nobody'"),
            "{err:#}"
        );
    }

    #[test]
    fn named_keys_resolve_and_dedup_against_literal_recipients() {
        let (dir, cfg) = fixture();
        let (a, b) = (pubkey(), pubkey());
        fs::write(
            &cfg,
            format!(
                "[keys]\nteo = [\"{a}\", \"{b}\"]\n[config]\n\"secrets/foo\" = [\"teo\", \"{a}\"]\n"
            ),
        )
        .unwrap();
        let app = AppConfig::load(&cfg, dir.path()).unwrap();
        assert_eq!(
            app.get_public_keys(&dir.path().join("secrets/foo"))
                .unwrap(),
            vec![a, b]
        );
    }

    #[test]
    fn add_named_accumulates_keys_and_saves_them_as_an_array() {
        let (dir, cfg) = fixture();
        let secret = dir.path().join("secrets/foo");
        fs::create_dir_all(secret.parent().unwrap()).unwrap();
        fs::write(&secret, "").unwrap();
        let (a, b) = (pubkey(), pubkey());
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add_named(
            "teo".into(),
            vec![a.clone()],
            vec![PathBuf::from("secrets/foo")],
        )
        .unwrap();
        app.save().unwrap();
        let text = fs::read_to_string(&cfg).unwrap();
        assert!(text.contains(&format!("teo = \"{a}\"")), "{text}");
        // Second recipient under the same name: One becomes Many, no dup.
        app.add_named(
            "teo".into(),
            vec![b.clone(), a.clone()],
            vec![PathBuf::from("secrets/foo")],
        )
        .unwrap();
        app.save().unwrap();
        let text = fs::read_to_string(&cfg).unwrap();
        assert!(
            text.contains(&format!("teo = [\"{a}\", \"{b}\"]")),
            "{text}"
        );
        let reloaded = AppConfig::load(&cfg, dir.path()).unwrap();
        assert_eq!(
            reloaded
                .get_public_keys(&dir.path().join("secrets/foo"))
                .unwrap(),
            vec![a, b]
        );
        // The ring lists the name once, not per call.
        assert_eq!(
            reloaded.list(),
            vec![("secrets/foo".to_string(), "teo".to_string())]
        );
    }

    #[test]
    fn save_places_keys_above_config_and_keeps_the_rest() {
        let (dir, cfg) = fixture();
        let (a, b) = (pubkey(), pubkey());
        let secret = dir.path().join("secrets/foo");
        fs::create_dir_all(secret.parent().unwrap()).unwrap();
        fs::write(&secret, "").unwrap();
        // [keys] after [config], a comment, and an unrelated table.
        fs::write(
            &cfg,
            format!(
                "# ring\n[config]\n\"secrets/foo\" = [\n  \"{a}\", # alice\n]\n\n[other]\nx = 1\n\n[keys]\nbob = \"{b}\"\n"
            ),
        )
        .unwrap();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(vec!["bob".into()], vec![PathBuf::from("secrets/foo")])
            .unwrap();
        app.save().unwrap();
        let text = fs::read_to_string(&cfg).unwrap();
        assert!(
            text.find("[keys]").unwrap() < text.find("[config]").unwrap(),
            "{text}"
        );
        assert!(text.contains("# ring"), "{text}");
        assert!(text.contains("[other]\nx = 1"), "{text}");
        assert!(text.contains(&format!("bob = \"{b}\"")), "{text}");
        let reloaded = AppConfig::load(&cfg, dir.path()).unwrap();
        assert_eq!(
            reloaded
                .get_public_keys(&dir.path().join("secrets/foo"))
                .unwrap(),
            vec![a, b]
        );
    }

    #[test]
    fn save_keeps_untouched_entries_byte_for_byte() {
        let (dir, cfg) = fixture();
        let (a, b) = (pubkey(), pubkey());
        for f in ["secrets/foo", "secrets/bar"] {
            let p = dir.path().join(f);
            fs::create_dir_all(p.parent().unwrap()).unwrap();
            fs::write(&p, "").unwrap();
        }
        let original =
            format!("[config]\n# multi-line stays\n\"secrets/foo\" = [\n  \"{a}\",\n]\n");
        fs::write(&cfg, &original).unwrap();
        let _g = CwdGuard::enter(dir.path());
        let mut app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.add(vec![b.clone()], vec![PathBuf::from("secrets/bar")])
            .unwrap();
        app.save().unwrap();
        let text = fs::read_to_string(&cfg).unwrap();
        assert!(text.starts_with(&original), "{text}");
        assert!(
            text.contains(&format!("\"secrets/bar\" = [\"{b}\"]")),
            "{text}"
        );
    }

    #[test]
    fn save_drops_an_empty_keys_table_and_creates_the_file_when_missing() {
        let (dir, cfg) = fixture();
        fs::write(&cfg, "[keys]\n[config]\n").unwrap();
        let app = AppConfig::load(&cfg, dir.path()).unwrap();
        app.save().unwrap();
        assert!(!fs::read_to_string(&cfg).unwrap().contains("[keys]"));
        // No file at all: save writes one.
        let fresh = dir.path().join("sub/git-agecrypt.toml");
        fs::create_dir_all(fresh.parent().unwrap()).unwrap();
        let app = AppConfig::load(&fresh, dir.path()).unwrap();
        app.save().unwrap();
        assert!(fresh.is_file());
    }

    #[test]
    fn load_and_save_report_unreadable_or_unwritable_files() {
        let (dir, _cfg) = fixture();
        // A directory where the file should be: neither readable nor writable
        // as a config file, and not "missing" either.
        let as_dir = dir.path().join("git-agecrypt.toml");
        fs::create_dir_all(&as_dir).unwrap();
        let err = AppConfig::load(&as_dir, dir.path())
            .err()
            .expect("a directory is not a readable config file");
        assert!(format!("{err:#}").contains("configuration file"), "{err:#}");
        let mut app = AppConfig::load(&dir.path().join("none.toml"), dir.path()).unwrap();
        app.path = as_dir;
        assert!(app.save().is_err());
        // Missing parent directory: the write itself fails.
        let mut app = AppConfig::load(&dir.path().join("none.toml"), dir.path()).unwrap();
        app.path = dir.path().join("missing-dir/git-agecrypt.toml");
        let err = app.save().unwrap_err();
        assert!(format!("{err:#}").contains("Couldn't save"), "{err:#}");
    }
}
