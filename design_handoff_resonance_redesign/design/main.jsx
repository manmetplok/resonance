/* Main app shell — wires views, transport, tweaks */

const TWEAK_DEFAULTS = /*EDITMODE-BEGIN*/{
  "view": "Arrange",
  "accent": "#8b6dff",
  "density": "balanced"
}/*EDITMODE-END*/;

function App() {
  const tw = useTweaks(TWEAK_DEFAULTS);
  const [view, setView] = React.useState(tw.view || "Arrange");
  const [playing, setPlaying] = React.useState(false);

  // sync tweak view → state
  React.useEffect(()=>{ if (tw.view && tw.view !== view) setView(tw.view); }, [tw.view]);

  // density variable
  const density = tw.density || "balanced";
  const rowH = density === "compact" ? 72 : density === "spacious" ? 116 : 96;

  // accent
  const accent = tw.accent || "#8b6dff";

  React.useEffect(() => {
    document.documentElement.style.setProperty("--accent", accent);
    document.documentElement.style.setProperty("--accent-soft", lighten(accent, 0.14));
    document.documentElement.style.setProperty("--accent-dim", hexA(accent, 0.16));
    document.documentElement.style.setProperty("--accent-line", hexA(accent, 0.34));
    document.documentElement.style.setProperty("--row-h", rowH + "px");
  }, [accent, rowH]);

  const onView = (v) => { setView(v); tw.set("view", v); };

  return (
    <>
      <Chrome view={view} setView={onView} accent={accent} />
      <Transport playing={playing} setPlaying={setPlaying} accent={accent} />
      <div style={{ flex: 1, minHeight: 0, overflow: "hidden" }}>
        {view === "Arrange" && <Arrange accent={accent} />}
        {view === "Mixer" && <Mixer accent={accent} />}
        {view === "Compose" && <Compose accent={accent} />}
      </div>

      <TweaksPanel title="Tweaks">
        <TweakSection title="Theme">
          <TweakColor t={tw} k="accent" label="Accent" options={["#8b6dff", "#e8c47b", "#6dd6a3", "#e8e7e3"]}/>
        </TweakSection>
        <TweakSection title="Layout">
          <TweakRadio t={tw} k="density" label="Density" options={["compact", "balanced", "spacious"]}/>
          <TweakRadio t={tw} k="view" label="View" options={["Arrange", "Mixer", "Compose"]}/>
        </TweakSection>
      </TweaksPanel>
    </>
  );
}

function hexA(hex, a) {
  const h = hex.replace("#","");
  const r = parseInt(h.substring(0,2),16);
  const g = parseInt(h.substring(2,4),16);
  const b = parseInt(h.substring(4,6),16);
  return `rgba(${r},${g},${b},${a})`;
}
function lighten(hex, amt) {
  const h = hex.replace("#","");
  let r = parseInt(h.substring(0,2),16);
  let g = parseInt(h.substring(2,4),16);
  let b = parseInt(h.substring(4,6),16);
  r = Math.min(255, Math.round(r + (255-r)*amt));
  g = Math.min(255, Math.round(g + (255-g)*amt));
  b = Math.min(255, Math.round(b + (255-b)*amt));
  return "#" + [r,g,b].map(x=>x.toString(16).padStart(2,"0")).join("");
}

function Mount() {
  return (
    <div style={{ display: "flex", flexDirection: "column", height: "100%" }}>
      <App />
    </div>
  );
}
ReactDOM.createRoot(document.getElementById("stage")).render(<Mount />);
