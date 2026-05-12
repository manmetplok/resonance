/* Resonance — redesigned shell */
const { useState, useMemo, useEffect, useRef } = React;

/* ───────── shared atoms ───────── */
const Icon = ({ d, size = 16, stroke = 1.5, fill = "none" }) => (
  <svg width={size} height={size} viewBox="0 0 24 24" fill={fill} stroke="currentColor" strokeWidth={stroke} strokeLinecap="round" strokeLinejoin="round">
    {typeof d === "string" ? <path d={d} /> : d}
  </svg>
);

const I = {
  play: <polygon points="6 4 20 12 6 20 6 4" fill="currentColor" stroke="none" />,
  pause: <g><rect x="6" y="4" width="4" height="16" fill="currentColor" stroke="none" /><rect x="14" y="4" width="4" height="16" fill="currentColor" stroke="none" /></g>,
  stop: <rect x="6" y="6" width="12" height="12" rx="1.5" fill="currentColor" stroke="none" />,
  rec: <circle cx="12" cy="12" r="6" fill="currentColor" stroke="none" />,
  prev: <g><polygon points="18 4 6 12 18 20 18 4" fill="currentColor" stroke="none"/><rect x="4" y="4" width="2" height="16" fill="currentColor" stroke="none" /></g>,
  next: <g><polygon points="6 4 18 12 6 20 6 4" fill="currentColor" stroke="none"/><rect x="18" y="4" width="2" height="16" fill="currentColor" stroke="none" /></g>,
  metronome: <g><path d="M9 4 L15 4 L18 20 L6 20 Z"/><path d="M12 19 L17 6"/></g>,
  loop: <g><path d="M4 12 a8 8 0 0 1 13.5 -5.7 L20 9"/><path d="M20 4 L20 9 L15 9"/><path d="M20 12 a8 8 0 0 1 -13.5 5.7 L4 15"/><path d="M4 20 L4 15 L9 15"/></g>,
  plus: <g><path d="M12 5 L12 19"/><path d="M5 12 L19 12"/></g>,
  more: <g><circle cx="5" cy="12" r="1" fill="currentColor"/><circle cx="12" cy="12" r="1" fill="currentColor"/><circle cx="19" cy="12" r="1" fill="currentColor"/></g>,
  mute: <g><path d="M11 5 L6 9 H3 v6 h3 l5 4 z"/><path d="M16 9 L21 14"/><path d="M21 9 L16 14"/></g>,
  solo: <path d="M12 4 L12 20 M6 8 L18 8 M6 16 L18 16"/>,
  arm: <circle cx="12" cy="12" r="4" fill="currentColor" stroke="none" />,
  headphones: <g><path d="M4 14 v3 a2 2 0 0 0 2 2 h2 v-7 H6 a2 2 0 0 0 -2 2 z"/><path d="M16 12 h2 a2 2 0 0 1 2 2 v3 a2 2 0 0 1 -2 2 h-2 z"/><path d="M4 14 a8 8 0 0 1 16 0"/></g>,
  freeze: <g><path d="M12 3 L12 21"/><path d="M3 12 L21 12"/><path d="M5.6 5.6 L18.4 18.4"/><path d="M18.4 5.6 L5.6 18.4"/></g>,
  link: <g><path d="M10 14 a4 4 0 0 1 0 -5.6 L13 5 a4 4 0 0 1 5.6 5.6 L17 12"/><path d="M14 10 a4 4 0 0 1 0 5.6 L11 19 a4 4 0 0 1 -5.6 -5.6 L7 12"/></g>,
  trash: <g><path d="M5 7 L19 7"/><path d="M9 7 V5 a1 1 0 0 1 1 -1 h4 a1 1 0 0 1 1 1 v2"/><path d="M7 7 L8 19 a1 1 0 0 0 1 1 h6 a1 1 0 0 0 1 -1 L17 7"/></g>,
  search: <g><circle cx="11" cy="11" r="6"/><path d="M16 16 L20 20"/></g>,
  chevron: <polyline points="6 9 12 15 18 9" />,
  sliders: <g><path d="M4 6 H20"/><circle cx="9" cy="6" r="2" fill="var(--bg-2)"/><path d="M4 12 H20"/><circle cx="15" cy="12" r="2" fill="var(--bg-2)"/><path d="M4 18 H20"/><circle cx="11" cy="18" r="2" fill="var(--bg-2)"/></g>,
  waveform: <g><path d="M3 12 H5 V8 V16 V12 H7 V4 V20 V12 H9 V9 V15 V12 H11 V6 V18 V12 H13 V10 V14 V12 H15 V7 V17 V12 H17 V11 V13 V12 H19 V8 V16 V12 H21"/></g>,
  midi: <g><rect x="3" y="9" width="18" height="6" rx="2"/><circle cx="8" cy="12" r="0.8" fill="currentColor"/><circle cx="12" cy="12" r="0.8" fill="currentColor"/><circle cx="16" cy="12" r="0.8" fill="currentColor"/></g>,
};

/* tiny SVG icon set wrapper */
const Ic = ({ k, size, stroke }) => <Icon d={I[k]} size={size} stroke={stroke} />;

/* ───────── window chrome + top bar ───────── */
function Chrome({ view, setView, accent }) {
  return (
    <div style={chromeS.root}>
      <div style={chromeS.left}>
        <div style={chromeS.dots}>
          <span style={{ ...chromeS.dot, background: "#ed6b5e" }} />
          <span style={{ ...chromeS.dot, background: "#f4be4f" }} />
          <span style={{ ...chromeS.dot, background: "#61c454" }} />
        </div>
        <div style={chromeS.title}>
          <span style={chromeS.brand}><span style={{ color: accent }}>●</span> Resonance</span>
          <span style={chromeS.sep}>/</span>
          <span style={chromeS.proj}><em style={{ fontFamily: "'Instrument Serif', serif", fontStyle: "italic", fontWeight: 400, fontSize: 17 }}>Glass Houses</em></span>
          <span style={chromeS.dirty}>· edited 2m ago</span>
        </div>
      </div>
      <nav style={chromeS.nav}>
        {["Arrange", "Mixer", "Compose"].map(v => (
          <button key={v} onClick={() => setView(v)}
            style={{ ...chromeS.tab, ...(view === v ? { color: "var(--text-1)", background: "var(--bg-3)" } : {}) }}>
            {v}
          </button>
        ))}
      </nav>
      <div style={chromeS.right}>
        <button style={chromeS.ghost}><Ic k="search" size={15} /> <span style={{opacity:.6}}>⌘K</span></button>
        <button style={chromeS.ghost}>Share</button>
        <div style={chromeS.avatar}>jw</div>
      </div>
    </div>
  );
}
const chromeS = {
  root: { display: "flex", alignItems: "center", justifyContent: "space-between", padding: "10px 14px", background: "var(--bg-1)", borderBottom: "1px solid var(--line-2)", height: 48, gap: 16 },
  left: { display: "flex", alignItems: "center", gap: 16 },
  dots: { display: "flex", gap: 6 },
  dot: { width: 11, height: 11, borderRadius: 999, display: "inline-block" },
  title: { display: "flex", alignItems: "baseline", gap: 8, fontSize: 13, color: "var(--text-2)" },
  brand: { color: "var(--text-1)", fontWeight: 500, letterSpacing: ".01em", display: "inline-flex", alignItems: "center", gap: 7 },
  sep: { color: "var(--text-4)" },
  proj: { color: "var(--text-1)" },
  dirty: { color: "var(--text-3)", fontSize: 12 },
  nav: { display: "flex", gap: 2, background: "var(--bg-2)", padding: 3, borderRadius: 8, border: "1px solid var(--line-2)" },
  tab: { fontSize: 12.5, padding: "6px 14px", border: "none", background: "transparent", color: "var(--text-2)", borderRadius: 6, cursor: "pointer", letterSpacing: ".02em", fontWeight: 500 },
  right: { display: "flex", alignItems: "center", gap: 8 },
  ghost: { display: "inline-flex", alignItems: "center", gap: 6, height: 28, padding: "0 10px", background: "transparent", border: "1px solid var(--line)", color: "var(--text-2)", borderRadius: 7, fontSize: 12, cursor: "pointer" },
  avatar: { width: 28, height: 28, borderRadius: 999, background: "linear-gradient(135deg,#8b6dff,#5e44d4)", display: "grid", placeItems: "center", fontSize: 11, fontWeight: 600, color: "white" },
};

/* ───────── transport row ───────── */
function Transport({ playing, setPlaying, accent }) {
  return (
    <div style={tpS.root}>
      <div style={tpS.left}>
        <button style={tpS.tBtn}><Ic k="prev" size={14} /></button>
        <button style={tpS.tBtn}><Ic k="stop" size={13} /></button>
        <button onClick={() => setPlaying(!playing)} style={{ ...tpS.tBtn, ...tpS.play, background: accent, borderColor: accent, color: "#0e0a1f" }}>
          <Ic k={playing ? "pause" : "play"} size={14} />
        </button>
        <button style={{ ...tpS.tBtn, color: "var(--bad)" }}><Ic k="rec" size={11} /></button>
        <button style={tpS.tBtn}><Ic k="next" size={14} /></button>
        <span style={tpS.div} />
        <button style={tpS.tBtn}><Ic k="loop" size={14} /></button>
        <button style={tpS.tBtn}><Ic k="metronome" size={14} /></button>
      </div>
      <div style={tpS.center}>
        <Stat label="POSITION" value="1.1.000" mono accent />
        <Stat label="TIME" value="00:00.000" mono />
        <Stat label="BPM" value="120" big mono />
        <Stat label="SIG" value="6/8" mono />
        <Stat label="KEY" value="B min" />
        <Stat label="LOOP" value="2 bars" mono />
      </div>
      <div style={tpS.right}>
        <Meter />
        <span style={tpS.cpu}>CPU 14%</span>
      </div>
    </div>
  );
}
function Stat({ label, value, big, mono, accent }) {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2, padding: "0 12px", borderRight: "1px solid var(--line-2)" }}>
      <span style={{ fontSize: 9.5, letterSpacing: ".14em", color: "var(--text-3)", textTransform: "uppercase" }}>{label}</span>
      <span style={{ fontSize: big ? 17 : 13, fontFamily: mono ? "'Geist Mono', monospace" : "Geist", fontWeight: 500, color: accent ? "var(--accent-soft)" : "var(--text-1)", fontVariantNumeric: "tabular-nums" }}>{value}</span>
    </div>
  );
}
function Meter() {
  return (
    <div style={{ display: "flex", flexDirection: "column", gap: 2 }}>
      {[0, 1].map(i => (
        <div key={i} style={{ width: 90, height: 4, background: "var(--bg-3)", borderRadius: 2, position: "relative", overflow: "hidden" }}>
          <div style={{ position: "absolute", left: 0, top: 0, bottom: 0, width: i === 0 ? "62%" : "48%", background: "linear-gradient(90deg, #6dd6a3 0%, #6dd6a3 70%, #e8c47b 90%, #e87b8b 100%)" }} />
        </div>
      ))}
    </div>
  );
}
const tpS = {
  root: { display: "flex", alignItems: "center", justifyContent: "space-between", padding: "10px 14px", background: "var(--bg-1)", borderBottom: "1px solid var(--line-2)", height: 56 },
  left: { display: "flex", alignItems: "center", gap: 6 },
  center: { display: "flex", alignItems: "center" },
  right: { display: "flex", alignItems: "center", gap: 14 },
  tBtn: { width: 32, height: 32, display: "grid", placeItems: "center", background: "var(--bg-2)", border: "1px solid var(--line)", color: "var(--text-1)", borderRadius: 8, cursor: "pointer" },
  play: { width: 36, height: 36 },
  div: { width: 1, height: 18, background: "var(--line)", margin: "0 4px" },
  cpu: { fontSize: 11, color: "var(--text-3)", fontFamily: "'Geist Mono', monospace" },
};

/* expose */
Object.assign(window, { Chrome, Transport, Ic, I, Icon });
