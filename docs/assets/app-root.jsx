// Root app + Tweaks integration

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "theme": "dark",
  "logoVariant": "shackle-node"
}/*EDITMODE-END*/;

function App() {
  const [tweaks, setTweak] = useTweaks(TWEAK_DEFAULTS);
  const [editMode, setEditMode] = useState(false);

  const theme = tweaks.theme;
  const logoVariant = tweaks.logoVariant;

  // Apply theme
  useEffect(() => {
    document.documentElement.setAttribute("data-theme", theme);
  }, [theme]);

  // Edit mode wiring
  useEffect(() => {
    const onMsg = (e) => {
      if (e.data?.type === "__activate_edit_mode") setEditMode(true);
      if (e.data?.type === "__deactivate_edit_mode") setEditMode(false);
    };
    window.addEventListener("message", onMsg);
    window.parent.postMessage({ type: "__edit_mode_available" }, "*");
    return () => window.removeEventListener("message", onMsg);
  }, []);

  return (
    <>
      <Nav logoVariant={logoVariant} theme={theme} setTheme={(v) => setTweak("theme", v)} />
      <Hero logoVariant={logoVariant} />
      <Features />
      <Quickstart />
      <Compare />
      <Docs />
      <HowItWorks />
      <CTA />
      <Footer logoVariant={logoVariant} />

      {editMode && (
        <TweaksPanel onClose={() => {
          setEditMode(false);
          window.parent.postMessage({ type: "__edit_mode_dismissed" }, "*");
        }}>
          <TweakSection title="Theme">
            <TweakRadio
              label="Mode"
              value={tweaks.theme}
              onChange={(v) => setTweak("theme", v)}
              options={[
                { value: "dark", label: "Dark" },
                { value: "light", label: "Light" },
              ]}
            />
          </TweakSection>
          <TweakSection title="Logo">
            <TweakSelect
              label="Variant"
              value={tweaks.logoVariant}
              onChange={(v) => setTweak("logoVariant", v)}
              options={[
                { value: "shackle-node", label: "Shackle + node" },
                { value: "keyhole-commit", label: "Keyhole + commit" },
                { value: "padlock-graph", label: "Padlock + graph" },
                { value: "glyph-a", label: "Glyph A (age)" },
              ]}
            />
            <div style={{
              display: "grid",
              gridTemplateColumns: "repeat(4, 1fr)",
              gap: 8,
              marginTop: 12,
            }}>
              {["shackle-node","keyhole-commit","padlock-graph","glyph-a"].map((v) => (
                <button
                  key={v}
                  onClick={() => setTweak("logoVariant", v)}
                  style={{
                    background: tweaks.logoVariant === v ? "var(--surface-2)" : "var(--surface)",
                    border: "1px solid " + (tweaks.logoVariant === v ? "var(--accent)" : "var(--line)"),
                    borderRadius: 8,
                    padding: 10,
                    cursor: "pointer",
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                  }}
                  title={v}
                >
                  <Logo variant={v} size={32} />
                </button>
              ))}
            </div>
          </TweakSection>
        </TweaksPanel>
      )}
    </>
  );
}

const root = ReactDOM.createRoot(document.getElementById("root"));
root.render(<App />);
