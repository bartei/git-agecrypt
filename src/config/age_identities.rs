use std::{fmt::Display, path::PathBuf};

use anyhow::Context as AnyhowContext;

use super::{Container, Result, Validated, git::GitConfigEntry};

pub(crate) struct AgeIdentity {
    pub path: String,
}

impl TryFrom<PathBuf> for AgeIdentity {
    type Error = anyhow::Error;

    fn try_from(value: PathBuf) -> std::result::Result<Self, Self::Error> {
        let s = value
            .to_str()
            .ok_or_else(|| anyhow::anyhow!("Unsupported path {:?}", &value))?;
        // Git config values cannot contain raw backslashes — libgit2 treats
        // them as escape introducers and rejects unknown sequences like
        // `\U` with `class=Config (7) invalid escape …`. Both libgit2 and
        // every age implementation accept forward slashes on Windows, so
        // normalise here and store a single canonical form regardless of
        // the host OS the entry was added on.
        let normalized = if cfg!(windows) {
            s.replace('\\', "/")
        } else {
            s.to_owned()
        };
        Ok(AgeIdentity { path: normalized })
    }
}

impl Display for AgeIdentity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.path.fmt(f)
    }
}

impl Validated for AgeIdentity {
    fn validate(&self) -> Result<()> {
        Ok(crate::age::validate_identity(&self.path)
            .with_context(|| format!("The file '{}' is not a valid age identity", self.path))?)
    }
}

pub(crate) struct AgeIdentities<C>(pub C)
where
    C: Container<Item = GitConfigEntry>;

impl<C> Container for AgeIdentities<C>
where
    C: Container<Item = GitConfigEntry>,
{
    type Item = AgeIdentity;

    fn add(&mut self, identity: Self::Item) -> Result<()> {
        identity.validate()?;
        self.0.add(identity.path.into())?;
        Ok(())
    }

    fn remove(&mut self, identity: Self::Item) -> Result<()> {
        self.0.remove(identity.path.into())?;
        Ok(())
    }

    fn list(&self) -> Result<Vec<Self::Item>> {
        let identities = self.0.list()?;
        Ok(identities
            .into_iter()
            .map(move |c| AgeIdentity { path: c.into() })
            .collect())
    }
}

impl<C> AgeIdentities<C>
where
    C: Container<Item = GitConfigEntry>,
{
    pub fn new(cfg: C) -> Self {
        Self(cfg)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn try_from_pathbuf_is_normalized_to_string() {
        // On non-Windows the round-trip is a straight `PathBuf` → `String`.
        // Just check that a UTF-8 path passes through.
        let id = AgeIdentity::try_from(PathBuf::from("/tmp/key")).unwrap();
        // Either unchanged on POSIX, or normalised forward-slash on Windows.
        if cfg!(windows) {
            assert!(!id.path.contains('\\'));
        } else {
            assert_eq!(id.path, "/tmp/key");
        }
    }

    #[test]
    fn try_from_normalises_windows_backslashes() {
        // Even on POSIX hosts, exercising the input path keeps `Display`
        // and the canonical-form policy in coverage.
        let id = AgeIdentity::try_from(PathBuf::from(r"C:\Users\test\key")).unwrap();
        if cfg!(windows) {
            assert_eq!(id.path, "C:/Users/test/key");
        } else {
            // POSIX treats backslashes as literal filename chars; nothing
            // to normalise.
            assert!(id.path.contains('\\'));
        }
    }

    #[test]
    fn display_renders_inner_path() {
        let id = AgeIdentity {
            path: "/tmp/foo".into(),
        };
        assert_eq!(format!("{id}"), "/tmp/foo");
    }

    #[test]
    fn validate_rejects_non_identity_file() {
        let dir = assert_fs::TempDir::new().unwrap();
        let p = dir.path().join("garbage");
        std::fs::write(&p, "not an identity").unwrap();
        let id = AgeIdentity {
            path: p.to_string_lossy().into_owned(),
        };
        assert!(id.validate().is_err());
    }
}
