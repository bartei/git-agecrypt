// How it works (clean / smudge / textconv flow), CTA, Footer

function HowItWorks() {
  return (
    <section className="section section-divider" id="how">
      <div className="container">
        <div className="section-head">
          <div>
            <div className="section-eyebrow">How it works</div>
            <h2 className="h2">Three filters, <em>one round trip.</em></h2>
          </div>
          <p className="section-sub">git-agecrypt plugs into git's clean / smudge / textconv filter mechanism. Plaintext lives in your working copy; ciphertext lives in the index and on the remote.</p>
        </div>

        <div className="flow">
          <div className="flow-row">
            <div className="flow-node work">
              <div className="flow-node-label">Working tree</div>
              <div className="flow-node-title">plaintext</div>
              <div className="flow-node-body">secrets/api-token</div>
            </div>
            <div className="flow-arrow">
              <svg width="44" height="14" viewBox="0 0 44 14" fill="none"><path d="M1 7H42M42 7L36 1M42 7L36 13" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/></svg>
              <span className="flow-arrow-label">clean</span>
              <span>git add</span>
            </div>
            <div className="flow-node">
              <div className="flow-node-label">Driver</div>
              <div className="flow-node-title">git-agecrypt clean</div>
              <div className="flow-node-body">stdin → stdout<br/>encrypts to recipients</div>
            </div>
            <div className="flow-arrow">
              <svg width="44" height="14" viewBox="0 0 44 14" fill="none"><path d="M1 7H42M42 7L36 1M42 7L36 13" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/></svg>
              <span className="flow-arrow-label">commit</span>
              <span>blob</span>
            </div>
            <div className="flow-node git">
              <div className="flow-node-label">Index / remote</div>
              <div className="flow-node-title">ciphertext</div>
              <div className="flow-node-body">age-encryption.org/v1</div>
            </div>
          </div>

          <div className="flow-row">
            <div className="flow-node git">
              <div className="flow-node-label">Index / remote</div>
              <div className="flow-node-title">ciphertext</div>
              <div className="flow-node-body">age-encryption.org/v1</div>
            </div>
            <div className="flow-arrow">
              <svg width="44" height="14" viewBox="0 0 44 14" fill="none"><path d="M1 7H42M42 7L36 1M42 7L36 13" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/></svg>
              <span className="flow-arrow-label">smudge</span>
              <span>git checkout</span>
            </div>
            <div className="flow-node">
              <div className="flow-node-label">Driver</div>
              <div className="flow-node-title">git-agecrypt smudge</div>
              <div className="flow-node-body">stdin → stdout<br/>decrypts using identity</div>
            </div>
            <div className="flow-arrow">
              <svg width="44" height="14" viewBox="0 0 44 14" fill="none"><path d="M1 7H42M42 7L36 1M42 7L36 13" stroke="currentColor" strokeWidth="1.4" strokeLinecap="round"/></svg>
              <span className="flow-arrow-label">write</span>
              <span>working tree</span>
            </div>
            <div className="flow-node work">
              <div className="flow-node-label">Working tree</div>
              <div className="flow-node-title">plaintext</div>
              <div className="flow-node-body">secrets/api-token</div>
            </div>
          </div>
        </div>

        <h4 style={{fontFamily: "var(--font-mono)", fontSize: 14, fontWeight: 500, color: "var(--accent)", margin: "44px 0 8px"}}>
          textconv
        </h4>
        <p style={{color: "var(--ink-2)", maxWidth: "70ch"}}>
          On <code>git diff</code> / <code>git log -p</code>, git invokes the textconv driver to decrypt encrypted files on the fly — your diffs always show plaintext, never opaque ciphertext blobs.
        </p>

        <h4 style={{fontFamily: "var(--font-mono)", fontSize: 14, fontWeight: 500, color: "var(--accent)", margin: "44px 0 8px"}}>
          Stable ciphertext via blake3 sidecars
        </h4>
        <p style={{color: "var(--ink-2)", maxWidth: "70ch"}}>
          age is non-deterministic — the same plaintext produces different ciphertext each time. Without mitigation, every <code>git status</code> would mark every encrypted file as modified. To avoid this, <code>clean</code> keeps two sidecar files per encrypted path under <code>.git/git-agecrypt/</code>:
        </p>
        <CodeBlock>{`.git/git-agecrypt/<encoded-path>.hash   # blake3 of last plaintext seen
.git/git-agecrypt/<encoded-path>.age    # ciphertext last produced for it`}</CodeBlock>
        <p style={{color: "var(--ink-2)", maxWidth: "70ch"}}>
          When <code>clean</code> runs, it hashes the incoming plaintext. If it matches the saved hash, the saved ciphertext is emitted verbatim — git sees no change. If it differs, <code>clean</code> compares against the decrypted HEAD; if those match, the HEAD ciphertext is reused. Only if both checks fail does the file get re-encrypted with fresh randomness.
        </p>

        <h4 style={{fontFamily: "var(--font-mono)", fontSize: 14, fontWeight: 500, color: "var(--accent)", margin: "44px 0 8px"}}>
          Where things are stored
        </h4>
        <table className="storage">
          <thead>
            <tr>
              <th>Location</th>
              <th>Contents</th>
              <th>Tracked in git?</th>
            </tr>
          </thead>
          <tbody>
            {window.STORAGE.map((row, i) => (
              <tr key={i}>
                <td>{row[0]}</td>
                <td style={{color: "var(--ink-2)"}}>{row[1]}</td>
                <td className={row[2] === "tracked" ? "tracked" : "untracked"}>{row[3]}</td>
              </tr>
            ))}
          </tbody>
        </table>

        <h4 style={{fontFamily: "var(--font-mono)", fontSize: 14, fontWeight: 500, color: "var(--accent)", margin: "44px 0 8px"}}>
          Limitations
        </h4>
        <ul style={{color: "var(--ink-2)", maxWidth: "70ch", paddingLeft: 20, lineHeight: 1.7}}>
          <li>The binary is re-executed once per file per git operation. Repos with thousands of encrypted files will see noticeable overhead.</li>
          <li>Whole files are loaded into memory during encrypt / decrypt. Don't use this for files that don't fit in RAM.</li>
          <li>Filters apply per-file, not per-directory. Use <code>secrets/**</code>, not <code>secrets/</code>, in <code>.gitattributes</code>.</li>
          <li>Removing a recipient does not rewrite history. Treat key revocation as a prompt to also rotate the underlying secrets.</li>
        </ul>
      </div>
    </section>
  );
}

function CTA() {
  return (
    <section className="cta">
      <div className="container">
        <div className="section-eyebrow" style={{justifyContent: "center", display: "flex"}}>Ship it</div>
        <h2 className="h2" style={{margin: "12px auto 0", textAlign: "center"}}>
          Stop committing secrets <em>in plaintext.</em>
        </h2>
        <p style={{color: "var(--muted)", maxWidth: "48ch", margin: "16px auto 0", textAlign: "center"}}>
          Open-source under MPL-2.0. One static binary. Works with the age ecosystem you already trust.
        </p>
        <div className="cta-buttons">
          <a className="btn btn-primary" href="#quickstart">Get started <span className="arr">→</span></a>
          <a className="btn btn-ghost" href="https://github.com/bartei/git-agecrypt" target="_blank" rel="noreferrer">View on GitHub</a>
        </div>
      </div>
    </section>
  );
}

function Footer({ logoVariant }) {
  return (
    <footer className="foot">
      <div className="container" style={{display: "flex", alignItems: "center", justifyContent: "space-between", flexWrap: "wrap", gap: 16}}>
        <Wordmark variant={logoVariant} />
        <div className="foot-links">
          <a href="https://github.com/bartei/git-agecrypt">GitHub</a>
          <a href="https://github.com/bartei/git-agecrypt/releases/latest">Releases</a>
          <a href="https://github.com/bartei/git-agecrypt/issues">Issues</a>
          <a href="https://age-encryption.org" target="_blank" rel="noreferrer">age</a>
          <a href="https://github.com/bartei/git-agecrypt/blob/main/LICENSE">MPL-2.0</a>
        </div>
        <div>built with rust · v0.3.0</div>
      </div>
    </footer>
  );
}

window.HowItWorks = HowItWorks;
window.CTA = CTA;
window.Footer = Footer;
