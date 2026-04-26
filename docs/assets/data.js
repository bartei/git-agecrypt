// Docs content — commands, workflows, and how-it-works data.

window.FEATURES = [
  {
    n: "01",
    title: "Transparent encryption",
    body: "git add, commit, push, pull, diff — all work as usual. Files are encrypted on the way into the index and decrypted on checkout.",
  },
  {
    n: "02",
    title: "Multi-recipient by path",
    body: "Encrypt different files to different recipient sets. Mappings live in a single committed git-agecrypt.toml.",
  },
  {
    n: "03",
    title: "Hardware keys, age plugins",
    body: "Use age x25519, OpenSSH ed25519/RSA, or any age plugin recipient — including age-plugin-yubikey for hardware-backed identities.",
  },
  {
    n: "04",
    title: "Stable ciphertext",
    body: "A blake3 sidecar makes the encrypted output deterministic relative to the working copy. git status doesn't churn untouched secrets.",
  },
  {
    n: "05",
    title: "Rotation friendly",
    body: "Add or remove recipients without rewriting history. The next commit re-encrypts only the files that actually changed.",
  },
  {
    n: "06",
    title: "Built in Rust",
    body: "Single static binary. No GPG keyring, no agents to babysit. Pre-built for Linux, macOS, Windows, and Nix.",
  },
];

window.COMPARE_ROWS = [
  ["Crypto primitive", "GPG (RSA / ECC)", "age (x25519, ed25519, plugins)"],
  ["Hardware keys", "GPG smartcard via gpg-agent", "YubiKey via age-plugin-yubikey, native"],
  ["Multi-recipient policy", "Per-repo, single key set", "Per-path, multiple key sets"],
  ["Recipient rotation", "Manual re-encryption", "Edit toml, touch files, commit"],
  ["Setup ceremony", "GPG keyring + trust dance", "One age key, one init"],
  ["Distribution", "C, system GPG", "Single static Rust binary"],
  ["Stable ciphertext", "Yes (deterministic)", "Yes (blake3 sidecar)"],
  ["Active maintenance", "Last release 2022", "Active"],
];

window.COMMANDS = [
  {
    name: "git-agecrypt init",
    tags: ["idempotent"],
    desc: "Wires git-agecrypt into the repo as clean / smudge / textconv driver in .git/config. Safe to re-run after moving the binary.",
    code: `$ git-agecrypt init`,
  },
  {
    name: "git-agecrypt deinit",
    tags: ["local-only"],
    desc: "Removes the filter wiring and clears the per-file ciphertext cache under .git/git-agecrypt/. Files committed encrypted stay encrypted in history.",
    code: `$ git-agecrypt deinit`,
  },
  {
    name: "git-agecrypt status",
    tags: [],
    desc: "Prints currently configured identities (your decryption keys) and recipients (encryption keys). ✓ means parses; ⨯ means misconfigured.",
    code: `$ git-agecrypt status
The following identities are currently configured:
    ✓ /home/alice/.config/age/personal.key

The following recipients are configured:
    secrets/api-token: age1qz5y…0p7w
    secrets/db.env:    age1qz5y…0p7w
    secrets/db.env:    age1jrnk…2qzp`,
  },
  {
    name: "config add -i <path>",
    tags: ["local-only"],
    desc: "Registers one of your private keys. Stored in .git/config — per-clone, never committed. You can register multiple identities.",
    code: `# Native age x25519 secret key file
$ git-agecrypt config add -i ~/.config/age/personal.key

# An OpenSSH ed25519 key
$ git-agecrypt config add -i ~/.ssh/id_ed25519

# A YubiKey-backed age plugin identity stub
$ git-agecrypt config add -i ~/.config/age/yubikey-stub.txt`,
  },
  {
    name: "config add -r <recipient> -p <path>...",
    tags: ["committed"],
    desc: "Registers a public key (recipient) for one or more paths. Mapping lives in git-agecrypt.toml — collaborators inherit it on clone.",
    code: `# Encrypt one file to one recipient
$ git-agecrypt config add \\
    -r "$(cat ~/.ssh/id_ed25519.pub)" \\
    -p secrets/api-token

# Encrypt several files to the same recipient
$ git-agecrypt config add \\
    -r age1qz5y…0p7w \\
    -p secrets/api-token secrets/db.env

# Encrypt one file to several recipients
$ git-agecrypt config add \\
    -r age1qz5y…0p7w \\
    -r age1jrnk…2qzp \\
    -p secrets/api-token`,
  },
  {
    name: "config remove -r <recipient>",
    tags: ["committed"],
    desc: "Drop a recipient from one or more paths. Does not rewrite history; takes effect on the next change to each file.",
    code: `# Drop bob from secrets/api-token only
$ git-agecrypt config remove \\
    -r age1jrnk…2qzp \\
    -p secrets/api-token

# Drop bob entirely
$ git-agecrypt config remove -r age1jrnk…2qzp`,
  },
  {
    name: "config list -i  ·  config list -r",
    tags: [],
    desc: "List configured identities (your private keys) or path → recipient mappings.",
    code: `$ git-agecrypt config list -i
The following identities are currently configured:
    ✓ /home/alice/.config/age/personal.key
    ✓ /home/alice/.ssh/id_ed25519

$ git-agecrypt config list -r
The following recipients are configured:
    secrets/api-token: age1qz5y…0p7w
    secrets/db.env:    age1qz5y…0p7w`,
  },
];

window.WORKFLOWS = [
  {
    title: "Onboard a collaborator",
    desc: "Bob shares his age public key. Grant him access to selected secrets — collaborators inherit recipients on clone.",
    code: `# 1. Add Bob's recipient to every file he should access
$ git-agecrypt config add \\
    -r age1jrnk…2qzp \\
    -p secrets/api-token secrets/db.env

# 2. Touch each affected file so the next commit re-encrypts
#    to the expanded recipient set
$ for f in secrets/api-token secrets/db.env; do
    cp "$f" "$f.tmp" && mv "$f.tmp" "$f"
  done

# 3. Commit and push
$ git add git-agecrypt.toml secrets/
$ git commit -m "grant bob access"
$ git push

# On Bob's machine
$ git clone <repo>
$ cd <repo>
$ git-agecrypt init
$ git-agecrypt config add -i ~/.config/age/bob.key
$ git checkout -- secrets/   # smudge filter decrypts on checkout`,
  },
  {
    title: "Off-board a collaborator",
    desc: "Revoke access for a recipient. Important: this controls current state, not history. Always rotate the underlying secret too.",
    code: `# Drop bob entirely
$ git-agecrypt config remove -r age1jrnk…2qzp

# Re-touch every encrypted file so it re-encrypts
# to the reduced recipient set
$ git ls-files | xargs -I {} sh -c \\
    'cp "{}" "{}.tmp" && mv "{}.tmp" "{}"'

$ git add -A && git commit -m "revoke bob"
$ git push

# THEN — actually rotate the secret
# (new API token, new DB password, etc.)`,
  },
  {
    title: "Use a YubiKey",
    desc: "Hardware-backed identity. Decryption requires the YubiKey to be plugged in and (optionally) touched.",
    code: `# 1. One-time setup — generates a hardware-backed identity slot
$ age-plugin-yubikey
# Pick "Generate a new identity"; note the printed
# recipient (age1yubikey1…) and the saved stub file.

# 2. Register the stub locally as your identity
$ git-agecrypt config add \\
    -i ~/.config/age/yubikey-stub.txt

# 3. Register the recipient so files get encrypted
#    to your YubiKey
$ git-agecrypt config add \\
    -r age1yubikey1… \\
    -p secrets/api-token`,
  },
  {
    title: "CI/CD deploy key",
    desc: "Dedicated keypair for CI. Inject the private key as a base64-encoded secret in your CI environment.",
    code: `# Local — generate the CI keypair
$ age-keygen -o ./ci.key
Public key: age1ci8m…lkpw

$ git-agecrypt config add \\
    -r age1ci8m…lkpw \\
    -p $(git ls-files secrets/)

# CI workflow (pseudo-code)
echo "$AGE_CI_KEY" > /tmp/ci.key
chmod 600 /tmp/ci.key
git-agecrypt init
git-agecrypt config add -i /tmp/ci.key
git checkout -- secrets/`,
  },
];

window.STORAGE = [
  ["git-agecrypt.toml", "path → recipient mappings", "tracked", "Yes — committed and shared"],
  [".gitattributes", "which paths use the filter", "tracked", "Yes — committed and shared"],
  [".git/config (filter)", "absolute path to the binary", "untracked", "No"],
  [".git/config (identity)", "paths to your private keys", "untracked", "No"],
  [".git/git-agecrypt/*.{hash,age}", "per-file ciphertext cache", "untracked", "No"],
];
