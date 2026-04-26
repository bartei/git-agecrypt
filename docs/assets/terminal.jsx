// Animated terminal — replays the git-agecrypt quickstart flow.

const TERMINAL_SCRIPT = [
  { kind: "comment", text: "1. Generate a personal age identity" },
  { kind: "cmd", text: "age-keygen -o ~/.config/age/personal.key" },
  { kind: "out", text: "Public key: age1qz5y…0p7w", delay: 380 },
  { kind: "cmd", text: "chmod 600 ~/.config/age/personal.key" },
  { kind: "spacer" },
  { kind: "comment", text: "2. Wire git-agecrypt into the repo" },
  { kind: "cmd", text: "cd ~/work/my-project" },
  { kind: "cmd", text: "git-agecrypt init" },
  { kind: "out", text: "→ filter installed in .git/config", delay: 240, dim: true },
  { kind: "spacer" },
  { kind: "comment", text: "3. Register your decryption identity (local-only)" },
  { kind: "cmd", text: "git-agecrypt config add -i ~/.config/age/personal.key" },
  { kind: "spacer" },
  { kind: "comment", text: "4. Encrypt a file to a recipient" },
  { kind: "cmd", text: "echo 'super-secret-token' > secrets/api-token" },
  { kind: "cmd", text: "git-agecrypt config add \\\n    -r age1qz5y…0p7w \\\n    -p secrets/api-token" },
  { kind: "spacer" },
  { kind: "comment", text: "5. Tell git which paths use the filter" },
  { kind: "cmd", text: "echo 'secrets/* filter=git-agecrypt diff=git-agecrypt' >> .gitattributes" },
  { kind: "spacer" },
  { kind: "comment", text: "6. Commit. Working copy = plaintext, blob = ciphertext." },
  { kind: "cmd", text: "git add .gitattributes git-agecrypt.toml secrets/api-token" },
  { kind: "cmd", text: "git commit -m \"encrypted api-token\"" },
  { kind: "out", text: "[main 8a4f1c2] encrypted api-token", delay: 260, dim: true },
  { kind: "out", text: " 3 files changed, 14 insertions(+)", delay: 260, dim: true },
  { kind: "spacer" },
  { kind: "comment", text: "Verify — git stores ciphertext only" },
  { kind: "cmd", text: "git show HEAD:secrets/api-token" },
  { kind: "out", text: "age-encryption.org/v1", delay: 260, accent: true },
  { kind: "out", text: "-> X25519 SclK7y/wHvL5h…", delay: 200, dim: true },
  { kind: "out", text: "Bgb2k5KkLZpQ4w1XY…", delay: 200, dim: true },
  { kind: "out", text: "--- yU3p4qQz8mE…", delay: 200, dim: true },
  { kind: "out", text: "0xß ▒▓Q1ŧ…⌬⊕…⌁ŋ⊻ƒ⊕⊗⊞", delay: 240, dim: true, glyphs: true },
];

function Terminal({ playing, speed = 1, onCycle }) {
  const [step, setStep] = React.useState(0);
  const [typed, setTyped] = React.useState("");
  const [phase, setPhase] = React.useState("typing"); // typing | output | done

  const cur = TERMINAL_SCRIPT[step];

  // Type-on effect for cmd lines
  React.useEffect(() => {
    if (!playing) return;
    if (!cur) {
      // loop
      const t = setTimeout(() => {
        setStep(0);
        setTyped("");
        onCycle && onCycle();
      }, 1800 / speed);
      return () => clearTimeout(t);
    }
    if (cur.kind === "cmd") {
      if (typed.length < cur.text.length) {
        const charDelay = cur.text[typed.length] === "\n" ? 80 : 14 + Math.random() * 22;
        const t = setTimeout(() => setTyped(cur.text.slice(0, typed.length + 1)), charDelay / speed);
        return () => clearTimeout(t);
      }
      const t = setTimeout(() => {
        setStep((s) => s + 1);
        setTyped("");
      }, 380 / speed);
      return () => clearTimeout(t);
    } else {
      // output / comment / spacer — just delay then advance
      const d = (cur.delay || (cur.kind === "spacer" ? 80 : 220)) / speed;
      const t = setTimeout(() => setStep((s) => s + 1), d);
      return () => clearTimeout(t);
    }
  }, [step, typed, playing, speed]);

  // Render history up to current step
  const history = TERMINAL_SCRIPT.slice(0, step);
  const scrollRef = React.useRef(null);
  React.useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [step, typed]);

  return (
    <div className="term">
      <div className="term-chrome">
        <div className="term-dots">
          <i></i><i></i><i></i>
        </div>
        <div className="term-title">~/work/my-project — zsh</div>
        <div className="term-meta">git-agecrypt · quickstart</div>
      </div>
      <div className="term-body" ref={scrollRef}>
        {history.map((line, i) => <Line key={i} line={line} />)}
        {cur && cur.kind === "cmd" && (
          <div className="term-line term-cmd">
            <span className="term-prompt">$</span>
            <span className="term-text">{typed.split("\n").map((seg, i, arr) => (
              <React.Fragment key={i}>
                {seg}
                {i < arr.length - 1 && <br />}
              </React.Fragment>
            ))}<span className="term-caret"></span></span>
          </div>
        )}
        {!cur && (
          <div className="term-line term-cmd">
            <span className="term-prompt">$</span>
            <span className="term-caret"></span>
          </div>
        )}
      </div>
    </div>
  );
}

function Line({ line }) {
  if (line.kind === "spacer") return <div className="term-spacer"></div>;
  if (line.kind === "comment") return <div className="term-line term-comment"># {line.text}</div>;
  if (line.kind === "cmd") {
    return (
      <div className="term-line term-cmd">
        <span className="term-prompt">$</span>
        <span className="term-text">{line.text.split("\n").map((seg, i, arr) => (
          <React.Fragment key={i}>
            {seg}
            {i < arr.length - 1 && <br />}
          </React.Fragment>
        ))}</span>
      </div>
    );
  }
  if (line.kind === "out") {
    const cls = ["term-line", "term-out", line.dim ? "dim" : "", line.accent ? "accent" : "", line.glyphs ? "glyphs" : ""].filter(Boolean).join(" ");
    return <div className={cls}>{line.text}</div>;
  }
  return null;
}

window.Terminal = Terminal;
