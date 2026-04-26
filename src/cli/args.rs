use std::path::PathBuf;

use clap::{ArgGroup, Parser, Subcommand};

/// Transparently encrypt/decrypt age secrets
#[derive(Parser)]
#[clap(author, version, about)]
pub struct Args {
    #[clap(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
#[clap(
    after_help = "In addition to the above, The following subcommands are used from git filters:
    clean, smudge, textconv"
)]
pub enum Commands {
    #[command(flatten)]
    Public(PublicCommands),
    #[command(flatten)]
    Internal(InternalCommands),
}

#[derive(Subcommand)]
pub enum PublicCommands {
    /// Set-up repository for use with git-agecrypt
    Init,

    /// Display configuration status information
    Status,

    /// Configure encryption settings
    #[command(subcommand)]
    Config(ConfigCommands),

    /// Remove repository specific configuration
    Deinit,
}

#[derive(Subcommand)]
pub enum ConfigCommands {
    /// Add a configuration entry
    Add(AddConfig),

    /// Remove a configuration entry
    Remove(RemoveConfig),

    /// List configuration entries
    List(ConfigType),
}

#[derive(clap::Args)]
#[clap(group(
    ArgGroup::new("config")
        .args(&["identity", "recipient"])
        .required(true)
))]
#[clap(group(
    ArgGroup::new("rec")
        .args(&["recipient"])
        .requires("path")
))]
pub struct AddConfig {
    /// Identity usable for decryption
    #[arg(short, long, num_args = 1.., group = "config")]
    identity: Option<PathBuf>,

    /// Recipient for encryption
    #[arg(short, long, num_args = 1.., group = "config")]
    recipient: Option<Vec<String>>,

    /// Path to encrypt for the given recipient
    #[arg(short, long, num_args = 1..)]
    path: Option<Vec<PathBuf>>,
}

pub(crate) enum ModifyConfig {
    Identity(PathBuf),
    Recipient(Vec<PathBuf>, Vec<String>),
}

impl From<AddConfig> for ModifyConfig {
    fn from(val: AddConfig) -> Self {
        if let Some(identity) = val.identity {
            Self::Identity(identity)
        } else if let Some(recipients) = val.recipient {
            Self::Recipient(val.path.unwrap(), recipients)
        } else {
            // The `config` ArgGroup above is `required(true)` over
            // {identity, recipient}, so clap rejects invocations that
            // supply neither before we ever construct an `AddConfig`.
            unreachable!("clap ArgGroup `config` requires identity or recipient")
        }
    }
}

#[derive(clap::Args)]
#[clap(group(
    ArgGroup::new("config")
        .args(&["identity", "recipient"])
))]
#[clap(group(
    ArgGroup::new("target")
        .args(&["identity", "recipient", "path"])
        .required(true)
        .multiple(true)
))]
pub struct RemoveConfig {
    /// Identity usable for decryption
    #[clap(short, long, group = "config")]
    identity: Option<PathBuf>,

    /// Recipient for encryption
    #[clap(short, long, group = "config")]
    recipient: Option<Vec<String>>,

    /// Path to encrypt for the given recipient
    #[clap(short, long)]
    path: Option<Vec<PathBuf>>,
}

impl From<RemoveConfig> for ModifyConfig {
    fn from(val: RemoveConfig) -> Self {
        if let Some(identity) = val.identity {
            Self::Identity(identity)
        } else if let Some(recipients) = val.recipient {
            Self::Recipient(val.path.unwrap_or_default(), recipients)
        } else if let Some(paths) = val.path {
            Self::Recipient(paths, vec![])
        } else {
            // The `target` ArgGroup is `required(true)` over
            // {identity, recipient, path}, so clap rejects invocations
            // that supply none of them.
            unreachable!("clap ArgGroup `target` requires identity, recipient, or path")
        }
    }
}

#[derive(clap::Args)]
#[clap(group(
    ArgGroup::new("type")
        .args(&["identity", "recipient"])
        .required(true)
))]
pub struct ConfigType {
    /// Identity usable for decryption
    #[arg(short, long)]
    identity: bool,

    /// Recipient for encryption
    #[arg(short, long)]
    recipient: bool,
}

pub(crate) enum QueryConfig {
    Identities,
    Recipients,
}

impl From<ConfigType> for QueryConfig {
    fn from(val: ConfigType) -> Self {
        if val.identity {
            Self::Identities
        } else if val.recipient {
            Self::Recipients
        } else {
            // The `type` ArgGroup is `required(true)` over
            // {identity, recipient}, so clap rejects invocations that
            // supply neither flag.
            unreachable!("clap ArgGroup `type` requires identity or recipient")
        }
    }
}

#[derive(Subcommand)]
pub enum InternalCommands {
    /// Encrypt files for commit
    #[command(hide = true)]
    Clean {
        /// File to clean
        #[clap(short, long)]
        file: PathBuf,
    },

    /// Decrypt files from checkout
    #[command(hide = true)]
    Smudge {
        /// File to smudge
        #[clap(short, long)]
        file: PathBuf,
    },

    /// Decrypt files for diff
    #[command(hide = true)]
    Textconv {
        /// File to show
        path: PathBuf,
    },
}

pub fn parse_args() -> Args {
    Args::parse()
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::CommandFactory;

    #[test]
    fn args_clap_definition_is_valid() {
        // clap derive macros emit code that can panic at runtime if the
        // attribute soup is wrong (e.g. duplicate short flags, conflicting
        // groups). Catch this at test time rather than at first user invocation.
        Args::command().debug_assert();
    }

    #[test]
    fn add_config_with_identity_maps_to_identity_variant() {
        let parsed = Args::parse_from(["git-agecrypt", "config", "add", "-i", "/tmp/some.key"]);
        let Commands::Public(PublicCommands::Config(ConfigCommands::Add(add))) = parsed.command
        else {
            panic!("expected config add");
        };
        match ModifyConfig::from(add) {
            ModifyConfig::Identity(p) => assert_eq!(p, PathBuf::from("/tmp/some.key")),
            ModifyConfig::Recipient(..) => panic!("expected identity variant"),
        }
    }

    #[test]
    fn add_config_with_recipient_and_path_maps_to_recipient_variant() {
        let parsed = Args::parse_from([
            "git-agecrypt",
            "config",
            "add",
            "-r",
            "age1example",
            "-p",
            "secrets/a",
            "secrets/b",
        ]);
        let Commands::Public(PublicCommands::Config(ConfigCommands::Add(add))) = parsed.command
        else {
            panic!("expected config add");
        };
        match ModifyConfig::from(add) {
            ModifyConfig::Recipient(paths, recipients) => {
                assert_eq!(
                    paths,
                    vec![PathBuf::from("secrets/a"), PathBuf::from("secrets/b")]
                );
                assert_eq!(recipients, vec!["age1example".to_string()]);
            }
            ModifyConfig::Identity(_) => panic!("expected recipient variant"),
        }
    }

    #[test]
    fn add_config_recipient_without_path_is_rejected_by_clap() {
        // The `rec` ArgGroup makes -p required when -r is supplied.
        let result = Args::try_parse_from(["git-agecrypt", "config", "add", "-r", "age1example"]);
        assert!(result.is_err());
    }

    #[test]
    fn remove_config_with_identity_maps_to_identity_variant() {
        let parsed = Args::parse_from(["git-agecrypt", "config", "remove", "-i", "/tmp/some.key"]);
        let Commands::Public(PublicCommands::Config(ConfigCommands::Remove(rm))) = parsed.command
        else {
            panic!("expected config remove");
        };
        match ModifyConfig::from(rm) {
            ModifyConfig::Identity(p) => assert_eq!(p, PathBuf::from("/tmp/some.key")),
            ModifyConfig::Recipient(..) => panic!("expected identity variant"),
        }
    }

    #[test]
    fn remove_config_recipient_only_clears_globally() {
        // `config remove -r <r>` (no path) means "drop this recipient
        // everywhere", which is encoded as Recipient(empty paths, [r]).
        let parsed = Args::parse_from(["git-agecrypt", "config", "remove", "-r", "age1example"]);
        let Commands::Public(PublicCommands::Config(ConfigCommands::Remove(rm))) = parsed.command
        else {
            panic!("expected config remove");
        };
        match ModifyConfig::from(rm) {
            ModifyConfig::Recipient(paths, recipients) => {
                assert!(paths.is_empty());
                assert_eq!(recipients, vec!["age1example".to_string()]);
            }
            ModifyConfig::Identity(_) => panic!("expected recipient variant"),
        }
    }

    #[test]
    fn remove_config_path_only_drops_path_entirely() {
        // `config remove -p <p>` (no recipient) means "drop this path",
        // encoded as Recipient([p], empty recipients).
        let parsed = Args::parse_from(["git-agecrypt", "config", "remove", "-p", "secrets/a"]);
        let Commands::Public(PublicCommands::Config(ConfigCommands::Remove(rm))) = parsed.command
        else {
            panic!("expected config remove");
        };
        match ModifyConfig::from(rm) {
            ModifyConfig::Recipient(paths, recipients) => {
                assert_eq!(paths, vec![PathBuf::from("secrets/a")]);
                assert!(recipients.is_empty());
            }
            ModifyConfig::Identity(_) => panic!("expected recipient variant"),
        }
    }

    #[test]
    fn list_config_identity_maps_to_identities_query() {
        let parsed = Args::parse_from(["git-agecrypt", "config", "list", "-i"]);
        let Commands::Public(PublicCommands::Config(ConfigCommands::List(t))) = parsed.command
        else {
            panic!("expected config list");
        };
        assert!(matches!(QueryConfig::from(t), QueryConfig::Identities));
    }

    #[test]
    fn list_config_recipient_maps_to_recipients_query() {
        let parsed = Args::parse_from(["git-agecrypt", "config", "list", "-r"]);
        let Commands::Public(PublicCommands::Config(ConfigCommands::List(t))) = parsed.command
        else {
            panic!("expected config list");
        };
        assert!(matches!(QueryConfig::from(t), QueryConfig::Recipients));
    }

    #[test]
    fn list_config_requires_choice() {
        // The `type` ArgGroup is required — list without -i/-r must error.
        let result = Args::try_parse_from(["git-agecrypt", "config", "list"]);
        assert!(result.is_err());
    }

    #[test]
    fn remove_config_requires_target() {
        // Regression: prior to Phase 3, `config remove` with no flags
        // panicked at runtime ("Misconfigured config parser") because the
        // ArgGroup wasn't required. The `target` group now forces clap to
        // reject this at parse time.
        let result = Args::try_parse_from(["git-agecrypt", "config", "remove"]);
        assert!(result.is_err());
    }

    #[test]
    fn internal_clean_command_parses() {
        let parsed = Args::parse_from(["git-agecrypt", "clean", "-f", "secrets/a"]);
        let Commands::Internal(InternalCommands::Clean { file }) = parsed.command else {
            panic!("expected internal clean");
        };
        assert_eq!(file, PathBuf::from("secrets/a"));
    }

    #[test]
    fn internal_smudge_command_parses() {
        let parsed = Args::parse_from(["git-agecrypt", "smudge", "-f", "secrets/a"]);
        let Commands::Internal(InternalCommands::Smudge { file }) = parsed.command else {
            panic!("expected internal smudge");
        };
        assert_eq!(file, PathBuf::from("secrets/a"));
    }

    #[test]
    fn internal_textconv_command_parses() {
        let parsed = Args::parse_from(["git-agecrypt", "textconv", "secrets/a"]);
        let Commands::Internal(InternalCommands::Textconv { path }) = parsed.command else {
            panic!("expected internal textconv");
        };
        assert_eq!(path, PathBuf::from("secrets/a"));
    }
}
