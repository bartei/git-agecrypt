// SPDX-FileCopyrightText: 2021 László Vaskó <opensource@vlaci.email>
// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

use anyhow::Context as _;

use crate::{Result, git};

use crate::config::Validated;
use crate::git::Repository;
use crate::{cache::CacheEntry, config::AgeIdentity, ctx::Context, gitcli};

const PROCESS_KEY: &str = "filter.git-agecrypt.process";

pub(crate) struct CommandContext<C: Context> {
    ctx: C,
}

impl<C: Context> CommandContext<C> {
    pub fn new(ctx: C) -> Self {
        Self { ctx }
    }

    /// Wire the filter and diff drivers. With `process`, also the long-running
    /// filter: git prefers `filter.<driver>.process` over clean/smudge when
    /// both are set, so a plain `init` clears any earlier one.
    pub(crate) fn init(&self, process: bool) -> Result<()> {
        let exe = shell_quote(&self.ctx.current_exe()?);
        let repo = self.ctx.repo();
        ensure_state(repo.set_config("filter.git-agecrypt.required", "true"))?;
        ensure_state(
            repo.set_config("filter.git-agecrypt.smudge", &format!("{exe} smudge -f %f")),
        )?;
        ensure_state(repo.set_config("filter.git-agecrypt.clean", &format!("{exe} clean -f %f")))?;
        ensure_state(repo.set_config("diff.git-agecrypt.textconv", &format!("{exe} textconv")))?;
        if process {
            ensure_state(repo.set_config(PROCESS_KEY, &format!("{exe} process")))?;
        } else {
            for value in repo.list_config(&format!("^{}$", regex::escape(PROCESS_KEY)))? {
                ensure_state(repo.remove_config(PROCESS_KEY, &value))?;
            }
        }
        Ok(())
    }

    /// Force re-encryption to the configured recipients, from
    /// vlaci/git-agecrypt 0.6 (László Vaskó, MPL-2.0): mark the cache, then
    /// let `git add --renormalize` run clean on every target. `paths` are
    /// pathspecs relative to the current directory, as for git itself; all
    /// encrypted files when empty.
    pub(crate) fn rekey(&self, paths: Vec<PathBuf>) -> Result<()> {
        let root = self.ctx.repo().workdir().to_path_buf();
        let cwd = std::env::current_dir().context("Cannot determine current directory")?;
        let specs: Vec<String> = paths
            .iter()
            .map(|p| p.to_string_lossy().into_owned())
            .collect();
        let specs: Vec<&str> = specs.iter().map(String::as_str).collect();
        let mut targets: Vec<String> = gitcli::filtered(&root, &gitcli::tracked(&cwd, &specs)?)?
            .into_iter()
            .collect();
        targets.sort();
        if targets.is_empty() {
            anyhow::bail!("no tracked file under the git-agecrypt filter matches");
        }
        for rel in &targets {
            self.ctx
                .store_cache(&root.join(rel), &CacheEntry::force_reencrypt())?;
        }
        gitcli::run_with_paths(&root, &["add", "--renormalize"], &targets)?;
        for rel in &targets {
            println!("Re-encrypted {rel}");
        }
        Ok(())
    }

    pub(crate) fn deinit(&self) -> Result<()> {
        let repo = self.ctx.repo();
        ensure_state(repo.remove_config_section("filter.git-agecrypt"))?;
        ensure_state(repo.remove_config_section("diff.git-agecrypt"))?;

        self.ctx.remove_sidecar_files()?;
        Ok(())
    }

    pub(crate) fn list_identities(&self) -> Result<()> {
        self.print_identities()
    }

    pub(crate) fn status(&self) -> Result<()> {
        self.list_identities()?;
        println!();
        self.list_recipients()?;
        Ok(())
    }

    pub(crate) fn add_identity(&self, identity: PathBuf) -> Result<()> {
        self.ctx
            .age_identities()
            .add(AgeIdentity::try_from(identity)?)?;
        Ok(())
    }

    pub(crate) fn remove_identity(&self, identity: PathBuf) -> Result<()> {
        self.ctx
            .age_identities()
            .remove(AgeIdentity::try_from(identity)?)?;
        Ok(())
    }

    fn print_identities(&self) -> Result<()> {
        let identities = self.ctx.age_identities().list()?;

        let padding = identities.iter().map(|i| i.path.len()).max().unwrap_or(0);
        println!("The following identities are currently configured:");
        for i in &identities {
            if let Err(err) = i.validate() {
                println!("    ⨯ {:padding$} -- {:?}", i.path, err, padding = padding);
            } else {
                println!("    ✓ {}", i.path);
            }
        }
        Ok(())
    }

    pub fn add_recipients(&self, recipients: Vec<String>, paths: Vec<PathBuf>) -> Result<()> {
        let mut cfg = self.ctx.config()?;

        cfg.add(recipients, paths)?;

        cfg.save()?;
        Ok(())
    }

    pub fn add_named(
        &self,
        name: String,
        recipients: Vec<String>,
        paths: Vec<PathBuf>,
    ) -> Result<()> {
        let mut cfg = self.ctx.config()?;
        cfg.add_named(name, recipients, paths)?;
        cfg.save()?;
        Ok(())
    }

    pub fn remove_recipients(&self, recipients: Vec<String>, paths: Vec<PathBuf>) -> Result<()> {
        let mut cfg = self.ctx.config()?;
        cfg.remove(recipients, paths)?;
        cfg.save()?;
        Ok(())
    }

    pub fn list_recipients(&self) -> Result<()> {
        let cfg = self.ctx.config()?;
        let recipients = cfg.list();

        println!("The following recipients are configured:");
        for (p, r) in recipients {
            println!("    {p}: {r}");
        }
        Ok(())
    }
}

fn ensure_state(result: git::Result<()>) -> Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(err) => match err {
            git::Error::AlreadyExists(_) => Ok(()),
            git::Error::NotExist(_) => Ok(()),
            err => Err(anyhow::anyhow!(err)),
        },
    }
}

/// Wrap a path for embedding into a `git config filter.*` command line.
///
/// Git invokes filter commands through `sh -c`, so the stored string is
/// re-parsed by a POSIX shell (msys2 sh on Windows). Plain double quotes
/// handle spaces; we additionally backslash-escape `\` and `"` so paths
/// containing those characters round-trip safely.
fn shell_quote(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for ch in s.chars() {
        match ch {
            '\\' | '"' | '$' | '`' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ensure_state_tolerates_already_exists_and_not_exist() {
        assert!(ensure_state(Err(git::Error::AlreadyExists("k".into()))).is_ok());
        assert!(ensure_state(Err(git::Error::NotExist("k".into()))).is_ok());
        assert!(ensure_state(Ok(())).is_ok());
    }

    #[test]
    fn ensure_state_propagates_other_errors() {
        let err = ensure_state(Err(git::Error::Other(anyhow::anyhow!("boom")))).unwrap_err();
        assert!(format!("{err:#}").contains("boom"));
    }

    #[test]
    fn quote_wraps_simple_path() {
        assert_eq!(
            shell_quote("/usr/local/bin/git-agecrypt"),
            "\"/usr/local/bin/git-agecrypt\""
        );
    }

    #[test]
    fn quote_handles_spaces() {
        assert_eq!(
            shell_quote("/Users/Some Body/git-agecrypt"),
            "\"/Users/Some Body/git-agecrypt\"",
        );
    }

    #[test]
    fn quote_escapes_double_quote() {
        assert_eq!(shell_quote(r#"weird"name"#), r#""weird\"name""#);
    }

    #[test]
    fn quote_escapes_backslash_dollar_backtick() {
        // sh would otherwise interpolate `$VAR`, run a `command` substitution,
        // or treat `\` as an escape introducer.
        assert_eq!(shell_quote(r"a\b"), r#""a\\b""#);
        assert_eq!(shell_quote("$VAR"), r#""\$VAR""#);
        assert_eq!(shell_quote("`cmd`"), r#""\`cmd\`""#);
    }
}
