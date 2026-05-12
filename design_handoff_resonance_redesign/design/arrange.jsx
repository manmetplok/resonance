/* Arrange view — timeline */
const { useState: useStateA } = React;

const ARRANGE_TRACKS = [
  { name: "Drums", kind: "Kit · Resonance Drums", icon: "rhythm", clips: [{ start: 1, end: 7, label: "Pattern A", kind: "midi", density: 0.85 }] },
  { name: "Drums Bounce", kind: "Audio · Wav", icon: "audio", muted: true, clips: [{ start: 1, end: 6.4, label: "Drums bounce", kind: "audio" }] },
  { name: "Synth Bass", kind: "Resonance Wave", icon: "bass", clips: [{ start: 1, end: 7, label: "Bm progression", kind: "midi", density: 0.4 }] },
  { name: "Synth Pad", kind: "Resonance Wave", icon: "pad", clips: [{ start: 1, end: 7, label: "Pad", kind: "midi", density: 0.25 }] },
  { name: "Lead Synth", kind: "Resonance Wave", icon: "lead", clips: [{ start: 1, end: 7, label: "Motif", kind: "midi", density: 0.6 }] },
  { name: "Aux", kind: "Bus · Verb", icon: "fx", empty: true },
];

function Arrange({ accent }) {
  const [selected, setSelected] = useStateA(0);
  const bars = 8;
  return (
    <div style={arrS.root}>
      <div style={arrS.toolbar}>
        <div style={arrS.left}>
          <div style={arrS.section}>
            <span style={arrS.sectionDot} />
            <span style={arrS.sectionLabel}>INTRO</span>
            <span style={arrS.sectionRange}>1—8</span>
          </div>
          <div style={{ ...arrS.section, opacity: .55 }}>
            <span style={{ ...arrS.sectionDot, background: "var(--text-3)" }} />
            <span style={arrS.sectionLabel}>VERSE</span>
            <span style={arrS.sectionRange}>9—24</span>
          </div>
          <button style={arrS.addSection}><Ic k="plus" size={12} /> Section</button>
        </div>
        <div style={arrS.right}>
          <button style={arrS.tool}>Edit section</button>
          <button style={arrS.tool}>Export chords</button>
          <span style={{width:1,height:18,background:"var(--line)"}}/>
          <button style={arrS.tool}><Ic k="sliders" size={13}/> View</button>
        </div>
      </div>

      <div style={arrS.body}>
        <div style={arrS.tracksCol}>
          <div style={arrS.tracksHeader}>
            <span>Tracks</span>
            <button style={arrS.addTrack}><Ic k="plus" size={12} /></button>
          </div>
          {ARRANGE_TRACKS.map((t, i) => (
            <TrackHeader key={i} t={t} selected={selected === i} onClick={() => setSelected(i)} accent={accent} />
          ))}
        </div>

        <div style={arrS.timeline}>
          <Ruler bars={bars} />
          <div style={arrS.lanes}>
            {ARRANGE_TRACKS.map((t, i) => (
              <Lane key={i} t={t} bars={bars} selected={selected === i} accent={accent} />
            ))}
          </div>
          <Playhead pct={70} />
          <SectionBand bars={bars} />
        </div>
      </div>
    </div>
  );
}

function TrackHeader({ t, selected, onClick, accent }) {
  return (
    <div onClick={onClick} style={{ ...arrS.trackHead, ...(selected ? { background: "var(--bg-2)", borderLeftColor: accent } : {}) }}>
      <div style={arrS.thLeft}>
        <span style={arrS.thIcon}>{glyphFor(t.icon)}</span>
        <div style={{display:"flex",flexDirection:"column",gap:2,minWidth:0}}>
          <span style={arrS.thName}>{t.name}</span>
          <span style={arrS.thKind}>{t.kind}</span>
        </div>
      </div>
      <div style={arrS.thControls}>
        <button style={{ ...arrS.tBtn, color: t.muted ? "var(--bad)" : "var(--text-3)" }}>M</button>
        <button style={arrS.tBtn}>S</button>
        <button style={arrS.tBtn}><Ic k="rec" size={9} /></button>
        <button style={arrS.tBtn}><Ic k="headphones" size={11} /></button>
      </div>
    </div>
  );
}

function glyphFor(kind) {
  const map = {
    rhythm: <svg width="14" height="14" viewBox="0 0 14 14"><circle cx="4" cy="10" r="2.5" fill="currentColor"/><path d="M6.5 10 V3 L11 2 V8" stroke="currentColor" strokeWidth="1.2" fill="none"/><circle cx="10" cy="9" r="2" fill="currentColor"/></svg>,
    audio: <svg width="14" height="14" viewBox="0 0 14 14"><g stroke="currentColor" strokeLinecap="round" strokeWidth="1.2"><path d="M2 7 V7"/><path d="M4 7 V5"/><path d="M5 7 V3"/><path d="M6 7 V4"/><path d="M7 7 V2"/><path d="M8 7 V5"/><path d="M9 7 V3"/><path d="M10 7 V4"/><path d="M12 7 V7"/><path d="M2 8 V8"/><path d="M4 9 V11"/><path d="M5 9 V12"/><path d="M6 9 V10"/><path d="M7 9 V13"/><path d="M8 9 V11"/><path d="M9 9 V12"/><path d="M10 9 V10"/></g></svg>,
    bass: <svg width="14" height="14" viewBox="0 0 14 14"><path d="M2 11 Q5 11 5 8 V3 H10" fill="none" stroke="currentColor" strokeWidth="1.2"/><circle cx="3.5" cy="11" r="1.5" fill="currentColor"/></svg>,
    pad: <svg width="14" height="14" viewBox="0 0 14 14"><path d="M2 7 Q5 2 7 7 T12 7" fill="none" stroke="currentColor" strokeWidth="1.2"/></svg>,
    lead: <svg width="14" height="14" viewBox="0 0 14 14"><path d="M2 11 V3 H10" fill="none" stroke="currentColor" strokeWidth="1.2"/><circle cx="3.5" cy="11" r="1.5" fill="currentColor"/></svg>,
    fx: <svg width="14" height="14" viewBox="0 0 14 14"><circle cx="7" cy="7" r="5" fill="none" stroke="currentColor" strokeWidth="1.2"/><circle cx="7" cy="7" r="2" fill="currentColor"/></svg>,
  };
  return map[kind] || map.fx;
}

function Ruler({ bars }) {
  const cells = Array.from({ length: bars }, (_, i) => i + 1);
  return (
    <div style={arrS.ruler}>
      {cells.map(b => (
        <div key={b} style={arrS.rulerCell}>
          <span style={arrS.rulerNum}>{b}</span>
          <div style={arrS.rulerTicks}>
            {[0,1,2,3].map(t => <span key={t} style={{ ...arrS.tick, opacity: t===0?0:0.4 }} />)}
          </div>
        </div>
      ))}
    </div>
  );
}

function SectionBand({ bars }) {
  return (
    <div style={arrS.sectionBand}>
      <div style={{ ...arrS.sectionPill, left: 0, width: `${(8/bars)*100}%`, background: "var(--accent-dim)", color: "var(--accent-soft)", borderColor: "var(--accent-line)" }}>
        <span>Intro</span><span style={{opacity:.6}}>· 6/8 · 90 BPM</span>
      </div>
    </div>
  );
}

function Lane({ t, bars, selected, accent }) {
  return (
    <div style={{ ...arrS.lane, background: selected ? "rgba(139,109,255,.04)" : "transparent" }}>
      <GridLines bars={bars} />
      {!t.empty && t.clips.map((c, i) => (
        <Clip key={i} c={c} bars={bars} accent={accent} />
      ))}
    </div>
  );
}

function GridLines({ bars }) {
  return (
    <div style={arrS.gridLines}>
      {Array.from({length: bars}).map((_, i) => (
        <div key={i} style={{ ...arrS.gridLine, left: `${(i/bars)*100}%`, borderLeft: i===0 ? "none" : "1px solid var(--line-2)" }}/>
      ))}
    </div>
  );
}

function Clip({ c, bars, accent }) {
  const left = ((c.start-1)/bars) * 100;
  const width = ((c.end-c.start+1)/bars) * 100;
  const isAudio = c.kind === "audio";
  return (
    <div style={{
      position: "absolute", left: `${left}%`, width: `${width}%`,
      top: 8, bottom: 8,
      background: isAudio ? "rgba(232,196,123,.10)" : "rgba(139,109,255,.10)",
      border: `1px solid ${isAudio ? "rgba(232,196,123,.32)" : "rgba(139,109,255,.34)"}`,
      borderRadius: 8,
      overflow: "hidden",
      display: "flex", flexDirection: "column"
    }}>
      <div style={{ padding: "5px 9px 0", display: "flex", justifyContent: "space-between", alignItems: "center" }}>
        <span style={{ fontSize: 10.5, fontWeight: 500, color: isAudio ? "#e8c47b" : "var(--accent-soft)", letterSpacing:".01em" }}>{c.label}</span>
        <span style={{ fontSize: 9, color: "var(--text-3)", fontFamily: "'Geist Mono',monospace" }}>{c.end-c.start+1}b</span>
      </div>
      <div style={{ flex: 1, position: "relative", padding: "4px 6px" }}>
        {isAudio ? <Waveform color="#e8c47b" /> : <MidiPreview density={c.density} />}
      </div>
    </div>
  );
}

function Waveform({ color = "var(--accent-soft)" }) {
  const bars = 80;
  return (
    <svg width="100%" height="100%" viewBox={`0 0 ${bars} 30`} preserveAspectRatio="none">
      {Array.from({length: bars}).map((_,i) => {
        const h = 4 + Math.abs(Math.sin(i*0.6)*12) + (i%7===0?6:0);
        return <rect key={i} x={i+0.2} y={15-h/2} width={0.6} height={h} fill={color} opacity={0.7}/>;
      })}
    </svg>
  );
}

function MidiPreview({ density = 0.5 }) {
  const notes = [];
  const seed = density * 100;
  for (let i = 0; i < 32; i++) {
    const x = (i / 32) * 100;
    const yJitter = ((i * 7 + seed) % 5) - 2;
    if (((i * 13 + seed) % 5) > (5 - density*5)) continue;
    notes.push({ x, y: 50 + yJitter * 8, w: 2 + ((i*3) % 3) });
  }
  return (
    <svg width="100%" height="100%" viewBox="0 0 100 100" preserveAspectRatio="none">
      {notes.map((n, i) => (
        <rect key={i} x={n.x} y={n.y} width={n.w} height={6} fill="var(--accent-soft)" opacity={0.85} rx={1}/>
      ))}
    </svg>
  );
}

function Playhead({ pct }) {
  return (
    <div style={{ position: "absolute", top: 0, bottom: 0, left: `calc(${pct}% + 0px)`, pointerEvents: "none", zIndex: 5 }}>
      <div style={{ width: 1, height: "100%", background: "var(--warm)" }}/>
      <div style={{ position: "absolute", top: -2, left: -5, width: 11, height: 11, background: "var(--warm)", borderRadius: "0 0 6px 6px" }}/>
    </div>
  );
}

const arrS = {
  root: { display: "flex", flexDirection: "column", height: "100%", background: "var(--bg-1)" },
  toolbar: { display: "flex", justifyContent: "space-between", alignItems: "center", padding: "10px 16px", borderBottom: "1px solid var(--line-2)", height: 44 },
  left: { display: "flex", alignItems: "center", gap: 14 },
  right: { display: "flex", alignItems: "center", gap: 6 },
  section: { display: "flex", alignItems: "center", gap: 7, fontSize: 11.5, color: "var(--text-1)", padding: "4px 10px", borderRadius: 6, background: "var(--bg-2)", border: "1px solid var(--line)" },
  sectionDot: { width: 7, height: 7, borderRadius: 999, background: "var(--accent)" },
  sectionLabel: { fontWeight: 600, letterSpacing: ".06em" },
  sectionRange: { color: "var(--text-3)", fontFamily: "'Geist Mono',monospace", fontSize: 11 },
  addSection: { display: "inline-flex", alignItems: "center", gap: 5, padding: "4px 10px", border: "1px dashed var(--line)", background: "transparent", color: "var(--text-3)", borderRadius: 6, fontSize: 11.5, cursor: "pointer" },
  tool: { display: "inline-flex", alignItems: "center", gap: 6, padding: "5px 11px", background: "transparent", border: "1px solid transparent", color: "var(--text-2)", borderRadius: 6, fontSize: 12, cursor: "pointer" },
  body: { flex: 1, display: "grid", gridTemplateColumns: "260px 1fr", overflow: "hidden" },
  tracksCol: { borderRight: "1px solid var(--line-2)", display: "flex", flexDirection: "column", overflow: "auto" },
  tracksHeader: { display: "flex", justifyContent: "space-between", alignItems: "center", padding: "10px 14px 6px", fontSize: 10.5, letterSpacing: ".14em", textTransform: "uppercase", color: "var(--text-3)" },
  addTrack: { width: 22, height: 22, display: "grid", placeItems: "center", background: "transparent", border: "1px dashed var(--line)", color: "var(--text-3)", borderRadius: 5, cursor: "pointer" },
  trackHead: { display: "flex", alignItems: "center", justifyContent: "space-between", padding: "0 14px", height: "var(--row-h)", borderBottom: "1px solid var(--line-2)", borderLeft: "2px solid transparent", cursor: "pointer", gap: 8 },
  thLeft: { display: "flex", alignItems: "center", gap: 10, minWidth: 0, flex: 1 },
  thIcon: { width: 28, height: 28, display: "grid", placeItems: "center", background: "var(--bg-2)", borderRadius: 7, color: "var(--text-2)" },
  thName: { fontSize: 13, fontWeight: 500, color: "var(--text-1)", whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" },
  thKind: { fontSize: 10.5, color: "var(--text-3)" },
  thControls: { display: "flex", gap: 3 },
  tBtn: { width: 19, height: 19, display: "grid", placeItems: "center", background: "transparent", border: "none", color: "var(--text-3)", fontSize: 9.5, fontWeight: 600, fontFamily: "'Geist Mono',monospace", borderRadius: 4, cursor: "pointer" },
  timeline: { position: "relative", overflow: "auto" },
  ruler: { display: "flex", height: 28, borderBottom: "1px solid var(--line-2)", position: "sticky", top: 0, background: "var(--bg-1)", zIndex: 4 },
  rulerCell: { flex: 1, padding: "5px 7px", borderLeft: "1px solid var(--line-2)", display: "flex", flexDirection: "column", justifyContent: "space-between" },
  rulerNum: { fontSize: 10, color: "var(--text-3)", fontFamily: "'Geist Mono',monospace" },
  rulerTicks: { display: "flex", justifyContent: "space-between", height: 4 },
  tick: { width: 1, background: "var(--text-4)", height: "100%" },
  sectionBand: { position: "absolute", top: 28, left: 0, right: 0, height: 22, display: "flex", borderBottom: "1px solid var(--line-2)" },
  sectionPill: { position: "absolute", top: 4, height: 14, display: "flex", alignItems: "center", gap: 8, padding: "0 9px", borderRadius: 4, fontSize: 10, border: "1px solid", letterSpacing:".05em" },
  lanes: { paddingTop: 22 },
  lane: { position: "relative", height: "var(--row-h)", borderBottom: "1px solid var(--line-2)" },
  gridLines: { position: "absolute", inset: 0 },
  gridLine: { position: "absolute", top: 0, bottom: 0, width: 0 },
};

window.Arrange = Arrange;
