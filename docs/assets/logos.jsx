// git-agecrypt logo variations
// Abstract mark: lock + git node motifs. All stroke-based, monoline, geometric.

const Logo = ({ variant = "shackle-node", size = 40, accent = "var(--accent)", ink = "currentColor", strokeWidth = 1.6 }) => {
  const s = size;

  // Variant A: Shackle-Node — padlock shackle becomes a git branch with three nodes
  if (variant === "shackle-node") {
    return (
      <svg width={s} height={s} viewBox="0 0 40 40" fill="none" xmlns="http://www.w3.org/2000/svg" aria-label="git-agecrypt logo">
        {/* shackle arc, open at bottom */}
        <path d="M10 20 V14 a10 10 0 0 1 20 0 V20" stroke={ink} strokeWidth={strokeWidth} strokeLinecap="round" />
        {/* git branch line — vertical spine through the lock body */}
        <line x1="20" y1="20" x2="20" y2="36" stroke={ink} strokeWidth={strokeWidth} strokeLinecap="round" />
        {/* branch off to the right */}
        <path d="M20 28 q4 0 4 -4 V22" stroke={accent} strokeWidth={strokeWidth} strokeLinecap="round" fill="none" />
        {/* nodes */}
        <circle cx="20" cy="20" r="2.4" fill={accent} />
        <circle cx="24" cy="22" r="1.8" fill={ink} stroke={accent} strokeWidth={strokeWidth * 0.6} />
        <circle cx="20" cy="36" r="1.8" fill={ink} stroke={ink} strokeWidth={strokeWidth} />
      </svg>
    );
  }

  // Variant B: Keyhole-Commit — a stylized keyhole where the opening is a commit dot
  if (variant === "keyhole-commit") {
    return (
      <svg width={s} height={s} viewBox="0 0 40 40" fill="none" xmlns="http://www.w3.org/2000/svg" aria-label="git-agecrypt logo">
        {/* outer rounded square — lock body */}
        <rect x="6" y="6" width="28" height="28" rx="6" stroke={ink} strokeWidth={strokeWidth} />
        {/* keyhole dot */}
        <circle cx="20" cy="17" r="3" fill={accent} />
        {/* keyhole tail — git line */}
        <line x1="20" y1="20" x2="20" y2="28" stroke={ink} strokeWidth={strokeWidth * 1.4} strokeLinecap="round" />
        {/* commit ticks on the line */}
        <circle cx="20" cy="28" r="1.2" fill={ink} />
      </svg>
    );
  }

  // Variant C: Padlock-Graph — bracket-style padlock with explicit git graph inside
  if (variant === "padlock-graph") {
    return (
      <svg width={s} height={s} viewBox="0 0 40 40" fill="none" xmlns="http://www.w3.org/2000/svg" aria-label="git-agecrypt logo">
        {/* shackle */}
        <path d="M12 18 V13 a8 8 0 0 1 16 0 V18" stroke={ink} strokeWidth={strokeWidth} strokeLinecap="round" />
        {/* body */}
        <rect x="8" y="18" width="24" height="16" rx="3" stroke={ink} strokeWidth={strokeWidth} />
        {/* graph inside: two parallel branches with a merge */}
        <path d="M14 22 V30 M20 22 q0 4 6 4 V30" stroke={accent} strokeWidth={strokeWidth} strokeLinecap="round" fill="none" />
        <circle cx="14" cy="22" r="1.4" fill={accent} />
        <circle cx="20" cy="22" r="1.4" fill={accent} />
        <circle cx="26" cy="30" r="1.4" fill={accent} />
      </svg>
    );
  }

  // Variant D: Glyph-A — Stylized 'a' (age) inside a shackle ring
  if (variant === "glyph-a") {
    return (
      <svg width={s} height={s} viewBox="0 0 40 40" fill="none" xmlns="http://www.w3.org/2000/svg" aria-label="git-agecrypt logo">
        {/* shackle ring */}
        <circle cx="20" cy="20" r="14" stroke={ink} strokeWidth={strokeWidth} />
        {/* gap at top representing open shackle / opening */}
        <line x1="17" y1="6.2" x2="23" y2="6.2" stroke="var(--bg)" strokeWidth={strokeWidth * 2} />
        {/* lowercase a — geometric */}
        <path d="M16 22 a4 4 0 1 0 0 -4 a4 4 0 1 0 0 4 z" stroke={accent} strokeWidth={strokeWidth} fill="none" />
        <line x1="20" y1="16" x2="20" y2="24" stroke={accent} strokeWidth={strokeWidth} strokeLinecap="round" />
        <line x1="20" y1="24" x2="24" y2="24" stroke={accent} strokeWidth={strokeWidth} strokeLinecap="round" />
      </svg>
    );
  }

  return null;
};

const Wordmark = ({ variant = "shackle-node", showWordmark = true }) => (
  <span className="wm">
    <Logo variant={variant} size={28} />
    {showWordmark && (
      <span className="wm-text">
        git<span className="wm-dim">-</span>agecrypt
      </span>
    )}
  </span>
);

window.Logo = Logo;
window.Wordmark = Wordmark;
