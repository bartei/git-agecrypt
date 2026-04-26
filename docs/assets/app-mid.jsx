// Quickstart, Docs, How-it-works, CTA, Footer

function Quickstart() {
  const [tab, setTab] = useState("cargo");
  const installs = {
    cargo: `$ cargo install --git https://github.com/bartei/git-agecrypt
$ git-agecrypt --version`,
    nix: `$ nix profile install github:bartei/git-agecrypt
$ git-agecrypt --version`,
    binary: `# Linux x86_64 (musl, static), install to ~/.local/bin
$ curl -L https://github.com/bartei/git-agecrypt/releases/latest/download/\\
    git-agecrypt-vX.Y.Z-x86_64-unknown-linux-musl.tar.gz \\
    | tar -xz -C ~/.local/bin
$ git-agecrypt --version`,
    source: `$ git clone https://github.com/bartei/git-agecrypt
$ cd git-agecrypt
$ cargo install --path .`,
  };
  return (
    <section className="section section-divider" id="quickstart">
      <div className="container">
        <div className="section-head">
          <div>
            <div className="section-eyebrow">Quickstart · 5 min</div>
            <h2 className="h2">Encrypted in <em>seven commands.</em></h2>
          </div>
          <p className="section-sub">This walkthrough encrypts <code>secrets/api-token</code> for yourself, using a fresh age x25519 keypair.</p>
        </div>

        <h3 style={{margin: "0 0 6px", fontSize: 18, fontWeight: 500, letterSpacing: "-0.01em"}}>
          <span style={{color: "var(--muted)", fontFamily: "var(--font-mono)", fontSize: 12, marginRight: 10}}>00</span>
          Install
        </h3>
        <div className="install-tabs">
          {["cargo", "nix", "binary", "source"].map((t) => (
            <button key={t} className={"install-tab" + (tab === t ? " active" : "")} onClick={() => setTab(t)}>
              {t}
            </button>
          ))}
        </div>
        <CodeBlock>{installs[tab]}</CodeBlock>

        <h3 style={{margin: "32px 0 6px", fontSize: 18, fontWeight: 500, letterSpacing: "-0.01em"}}>
          <span style={{color: "var(--muted)", fontFamily: "var(--font-mono)", fontSize: 12, marginRight: 10}}>01</span>
          Generate an age identity
        </h3>
        <p style={{color: "var(--ink-2)", maxWidth: "60ch", margin: "0 0 4px"}}>
          Treat the resulting file like an SSH private key — it's the only thing that can decrypt the repo's secrets.
        </p>
        <CodeBlock>{`$ age-keygen -o ~/.config/age/personal.key
Public key: age1qz5y…0p7w
$ chmod 600 ~/.config/age/personal.key`}</CodeBlock>

        <h3 style={{margin: "32px 0 6px", fontSize: 18, fontWeight: 500, letterSpacing: "-0.01em"}}>
          <span style={{color: "var(--muted)", fontFamily: "var(--font-mono)", fontSize: 12, marginRight: 10}}>02</span>
          Wire it into your repo
        </h3>
        <CodeBlock>{`$ cd ~/work/my-project
$ git-agecrypt init
$ git-agecrypt config add -i ~/.config/age/personal.key`}</CodeBlock>

        <h3 style={{margin: "32px 0 6px", fontSize: 18, fontWeight: 500, letterSpacing: "-0.01em"}}>
          <span style={{color: "var(--muted)", fontFamily: "var(--font-mono)", fontSize: 12, marginRight: 10}}>03</span>
          Encrypt a file
        </h3>
        <CodeBlock>{`$ mkdir -p secrets
$ printf 'super-secret-token\\n' > secrets/api-token

$ git-agecrypt config add \\
    -r age1qz5y…0p7w \\
    -p secrets/api-token

$ echo 'secrets/* filter=git-agecrypt diff=git-agecrypt' >> .gitattributes`}</CodeBlock>

        <h3 style={{margin: "32px 0 6px", fontSize: 18, fontWeight: 500, letterSpacing: "-0.01em"}}>
          <span style={{color: "var(--muted)", fontFamily: "var(--font-mono)", fontSize: 12, marginRight: 10}}>04</span>
          Commit. Verify.
        </h3>
        <CodeBlock>{`$ git add .gitattributes git-agecrypt.toml secrets/api-token
$ git commit -m "encrypted api-token"

# What does git actually see? (Should be ciphertext.)
$ git show HEAD:secrets/api-token
age-encryption.org/v1
-> X25519 SclK7y…
…`}</CodeBlock>

        <div className="callout">
          <strong>That's it.</strong> After cloning the repo on another machine, run <code>git-agecrypt init</code> and <code>git-agecrypt config add -i &lt;your-key&gt;</code> — recipients are already in <code>git-agecrypt.toml</code>. <code>git checkout</code> decrypts back to plaintext automatically.
        </div>
      </div>
    </section>
  );
}

// ---------- Docs ----------
function Docs() {
  return (
    <section className="section section-divider" id="docs">
      <div className="container">
        <div className="section-head">
          <div>
            <div className="section-eyebrow">Reference</div>
            <h2 className="h2">Commands &amp; <em>workflows.</em></h2>
          </div>
          <p className="section-sub">Every public subcommand. Print contextual help for any of them with <code>git-agecrypt &lt;cmd&gt; --help</code>.</p>
        </div>

        <div className="doc">
          <aside className="doc-toc">
            <div className="doc-toc-group">Commands</div>
            <a href="#cmd-init">init</a>
            <a href="#cmd-deinit">deinit</a>
            <a href="#cmd-status">status</a>
            <a href="#cmd-id-add">config add -i</a>
            <a href="#cmd-rec-add">config add -r</a>
            <a href="#cmd-rec-rm">config remove -r</a>
            <a href="#cmd-list">config list</a>
            <div className="doc-toc-group">Workflows</div>
            <a href="#wf-onboard">Onboard</a>
            <a href="#wf-offboard">Off-board</a>
            <a href="#wf-yubikey">YubiKey</a>
            <a href="#wf-ci">CI/CD</a>
          </aside>

          <div className="doc-content">
            <div className="doc-section">
              <h3 id="cmd-ref">Commands</h3>
              <p>Public commands mutate <code>.git/config</code> and the committed <code>git-agecrypt.toml</code>. Internal commands (<code>clean</code>, <code>smudge</code>, <code>textconv</code>) are invoked by git itself and hidden from <code>--help</code>.</p>
              {window.COMMANDS.map((c, i) => {
                const slug = ["cmd-init","cmd-deinit","cmd-status","cmd-id-add","cmd-rec-add","cmd-rec-rm","cmd-list"][i];
                return (
                  <div className="cmd-card" id={slug} key={i}>
                    <div className="cmd-card-head">
                      <div className="cmd-card-name">{c.name}</div>
                      {c.tags.map((t) => (
                        <span key={t} className={"cmd-card-tag " + (t === "idempotent" ? "idem" : t === "local-only" ? "local" : "committed")}>
                          {t}
                        </span>
                      ))}
                    </div>
                    <p className="cmd-card-desc">{c.desc}</p>
                    <CodeBlock>{c.code}</CodeBlock>
                  </div>
                );
              })}
            </div>

            <div className="doc-section">
              <h3 id="workflows">Workflows</h3>
              <p>Recipes for the operations you'll actually run more than once.</p>
              {window.WORKFLOWS.map((w, i) => {
                const slug = ["wf-onboard","wf-offboard","wf-yubikey","wf-ci"][i];
                return (
                  <div key={i} id={slug} style={{marginTop: 28}}>
                    <h4>{w.title}</h4>
                    <p style={{color: "var(--ink-2)", margin: "0 0 8px"}}>{w.desc}</p>
                    <CodeBlock>{w.code}</CodeBlock>
                  </div>
                );
              })}
              <div className="callout" style={{marginTop: 20}}>
                <strong>Important:</strong> <code>git-agecrypt</code> controls the <em>current</em> state of the repository, not its history. After revoking, <strong>rotate the underlying secrets</strong> — that is the actual revocation.
              </div>
            </div>
          </div>
        </div>
      </div>
    </section>
  );
}

window.Quickstart = Quickstart;
window.Docs = Docs;
