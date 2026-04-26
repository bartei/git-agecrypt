// Main app — landing + docs + quickstart for git-agecrypt

const { useState, useEffect, useRef } = React;

// ---------- Helpers ----------
function highlight(code) {
  // Simple shell highlighter
  const lines = code.split("\n");
  return lines.map((line, i) => {
    let parts = [];
    let rest = line;
    if (rest.startsWith("# ") || rest.startsWith("#!")) {
      parts.push(<span key="c" className="c-comment">{rest}</span>);
    } else if (rest.startsWith("$ ")) {
      parts.push(<span key="p" className="c-prompt">$ </span>);
      rest = rest.slice(2);
      // tokenize remaining
      const tok = rest.split(/(\s+)/);
      tok.forEach((t, j) => {
        if (/^-[ipr-]+$/.test(t) || /^--[a-z-]+$/.test(t)) {
          parts.push(<span key={"t"+j} className="c-flag">{t}</span>);
        } else if (/^["'].*["']$/.test(t)) {
          parts.push(<span key={"t"+j} className="c-str">{t}</span>);
        } else {
          parts.push(<React.Fragment key={"t"+j}>{t}</React.Fragment>);
        }
      });
    } else if (rest.trim().startsWith("✓") || rest.trim().startsWith("⨯")) {
      parts.push(<span key="o" className="c-comment">{rest}</span>);
    } else {
      parts.push(<span key="o" className="c-comment">{rest}</span>);
    }
    return (
      <React.Fragment key={i}>
        {parts}
        {i < lines.length - 1 && "\n"}
      </React.Fragment>
    );
  });
}

function CodeBlock({ children }) {
  const [copied, setCopied] = useState(false);
  const onCopy = () => {
    navigator.clipboard.writeText(children).then(() => {
      setCopied(true);
      setTimeout(() => setCopied(false), 1400);
    });
  };
  return (
    <pre className="code">
      <button className={"code-copy" + (copied ? " copied" : "")} onClick={onCopy}>
        {copied ? "copied" : "copy"}
      </button>
      <code>{highlight(children)}</code>
    </pre>
  );
}

// ---------- Nav ----------
function Nav({ logoVariant, theme, setTheme }) {
  return (
    <nav className="nav">
      <div className="container nav-inner">
        <a href="#top">
          <Wordmark variant={logoVariant} />
        </a>
        <div className="nav-links">
          <a href="#features" className="nav-link-secondary">Features</a>
          <a href="#quickstart">Quickstart</a>
          <a href="#docs">Docs</a>
          <a href="#how" className="nav-link-optional">How it works</a>
          <a href="#compare" className="nav-link-optional">vs git-crypt</a>
          <button
            className="theme-toggle"
            onClick={() => setTheme(theme === "dark" ? "light" : "dark")}
            aria-label="Toggle theme"
            title="Toggle theme"
          >
            {theme === "dark" ? (
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><circle cx="12" cy="12" r="4"/><path d="M12 2v2M12 20v2M4.93 4.93l1.41 1.41M17.66 17.66l1.41 1.41M2 12h2M20 12h2M6.34 17.66l-1.41 1.41M19.07 4.93l-1.41 1.41"/></svg>
            ) : (
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2"><path d="M21 12.79A9 9 0 1 1 11.21 3 7 7 0 0 0 21 12.79z"/></svg>
            )}
          </button>
          <a className="nav-cta" href="https://github.com/bartei/git-agecrypt" target="_blank" rel="noreferrer">
            <svg width="13" height="13" viewBox="0 0 24 24" fill="currentColor"><path d="M12 .5C5.65.5.5 5.65.5 12c0 5.08 3.29 9.39 7.86 10.91.57.1.78-.25.78-.55v-2c-3.2.7-3.87-1.36-3.87-1.36-.52-1.32-1.27-1.67-1.27-1.67-1.04-.71.08-.7.08-.7 1.15.08 1.76 1.18 1.76 1.18 1.02 1.75 2.69 1.24 3.34.95.1-.74.4-1.24.72-1.53-2.55-.29-5.24-1.28-5.24-5.69 0-1.26.45-2.29 1.18-3.09-.12-.3-.51-1.46.11-3.04 0 0 .96-.31 3.15 1.18a10.94 10.94 0 0 1 5.74 0c2.19-1.49 3.15-1.18 3.15-1.18.62 1.58.23 2.74.12 3.04.74.8 1.18 1.83 1.18 3.09 0 4.42-2.69 5.39-5.25 5.68.41.36.78 1.06.78 2.14v3.17c0 .31.21.66.79.55C20.21 21.39 23.5 17.08 23.5 12 23.5 5.65 18.35.5 12 .5z"/></svg>
            github
          </a>
        </div>
      </div>
    </nav>
  );
}

// ---------- Hero ----------
function Hero({ logoVariant }) {
  return (
    <section className="hero" id="top">
      <div className="container hero-grid">
        <div>
          <div className="eyebrow fade-in"><span className="dot"></span>v0.3.0 · MPL-2.0 · Rust</div>
          <h1 className="h1 fade-in d1">
            Encrypted secrets,<br/><em>plain-text workflow.</em>
          </h1>
          <p className="lede fade-in d2">
            <strong>git-agecrypt</strong> is transparent file-level encryption for git repositories, powered by <a href="https://age-encryption.org" target="_blank" rel="noreferrer" style={{color:"var(--accent)"}}>age</a>. Plaintext stays in your working tree; ciphertext is what travels through <code>git add</code>, <code>git push</code>, and ends up in the remote.
          </p>
          <p className="lede fade-in d2" style={{marginTop: 12, fontSize: 15, color: "var(--muted)"}}>
            A modern, portable replacement for git-crypt — same workflow, but using age instead of GPG. x25519, OpenSSH ed25519, or any age plugin recipient (YubiKey PIV, etc.).
          </p>
          <div className="hero-cta fade-in d3">
            <a href="#quickstart" className="btn btn-primary">
              Quickstart <span className="arr">→</span>
            </a>
            <a href="#docs" className="btn btn-ghost">Read the docs</a>
          </div>
          <div className="hero-meta fade-in d4">
            <div className="hero-meta-item">
              <div className="hero-meta-label">Install</div>
              <div className="hero-meta-value">cargo install git-agecrypt</div>
            </div>
            <div className="hero-meta-item">
              <div className="hero-meta-label">Backed by</div>
              <div className="hero-meta-value">age-encryption.org/v1</div>
            </div>
            <div className="hero-meta-item">
              <div className="hero-meta-label">Platforms</div>
              <div className="hero-meta-value">linux · macos · windows · nix</div>
            </div>
          </div>
        </div>
        <div className="hero-art fade-in d2">
          <Terminal playing={true} speed={1} />
        </div>
      </div>
    </section>
  );
}

// ---------- Features ----------
function Features() {
  return (
    <section className="section section-divider" id="features">
      <div className="container">
        <div className="section-head">
          <div>
            <div className="section-eyebrow">Features</div>
            <h2 className="h2">Six things that make it <em>quietly powerful.</em></h2>
          </div>
          <p className="section-sub">Designed to disappear into your normal git workflow. No agents to start, no keyring to babysit, no rewriting of history.</p>
        </div>
        <div className="features">
          {window.FEATURES.map((f) => (
            <div className="feature" key={f.n}>
              <div className="feature-num">{f.n}</div>
              <div className="feature-title">{f.title}</div>
              <div className="feature-body">{f.body}</div>
            </div>
          ))}
        </div>
      </div>
    </section>
  );
}

// ---------- Compare ----------
function Compare() {
  const rows = window.COMPARE_ROWS;
  return (
    <section className="section section-divider" id="compare">
      <div className="container">
        <div className="section-head">
          <div>
            <div className="section-eyebrow">vs git-crypt</div>
            <h2 className="h2">Same workflow, <em>modern crypto.</em></h2>
          </div>
          <p className="section-sub">git-crypt was a great idea bound to GPG. git-agecrypt keeps the workflow and swaps in age — smaller, simpler, hardware-friendly.</p>
        </div>
        <div className="compare-wrap">
          <table className="compare">
            <thead>
              <tr>
                <th></th>
                <th>git-crypt</th>
                <th className="col-us-head">git-agecrypt</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((row, i) => (
                <tr key={i}>
                  <td>{row[0]}</td>
                  <td>{row[1]}</td>
                  <td className="col-us">{row[2]}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </div>
    </section>
  );
}

window.Nav = Nav;
window.Hero = Hero;
window.Features = Features;
window.Compare = Compare;
window.CodeBlock = CodeBlock;
