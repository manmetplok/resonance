/* Compose view — chord lane + instruments + drum grid */

const COMPOSE_CHORDS = [
  { sym: "Bm", deg: "i", bars: 1, locked: true },
  { sym: "Bm", deg: "i", bars: 1 },
  { sym: "F♯", deg: "V", bars: 1 },
  { sym: "G", deg: "VI", bars: 1 },
  { sym: "E", deg: "iv", bars: 1 },
  { sym: "Bm", deg: "i", bars: 1 },
  { sym: "Bm", deg: "i", bars: 1 },
  { sym: "—", deg: "", bars: 1, empty: true },
];

const DRUM_KIT = [
  { name: "Kick", color: "var(--accent-soft)", pattern: [1,0,0,1,0,0,1,0,0,0,1,0,1,0,0,1] },
  { name: "Snare", color: "#e8c47b", pattern: [0,0,0,0,1,0,0,0,0,0,1,0,0,0,1,0] },
  { name: "Clap", color: "#e8c47b", pattern: [0,0,0,0,1,0,0,0,0,0,1,0,0,0,0,0] },
  { name: "Hat C", color: "var(--text-2)", pattern: [1,1,0,1,1,1,0,1,1,1,0,1,1,1,0,1] },
  { name: "Hat O", color: "var(--text-2)", pattern: [0,0,1,0,0,0,1,0,0,0,1,0,0,0,1,0] },
  { name: "Tom",  color: "#a892ff", pattern: [0,0,0,0,0,0,0,0,1,0,0,0,0,0,0,0] },
  { name: "Perc", color: "#a892ff", pattern: [0,1,0,0,0,1,0,0,0,1,0,0,0,1,0,1] },
];

function Compose({ accent }) {
  const [section, setSection] = React.useState(0);
  return (
    <div style={cpS.root}>
      <div style={cpS.toolbar}>
        <div style={{display:"flex",alignItems:"center",gap:8}}>
          {[
            {label: "Intro", range: "1—8"},
            {label: "Verse", range: "9—24"},
            {label: "Chorus", range: "25—40"}
          ].map((s,i)=>(
            <button key={i} onClick={()=>setSection(i)}
              style={{ ...cpS.sectTab, ...(section===i ? cpS.sectTabOn : {}) }}>
              <span style={{...cpS.sectDot, background: section===i ? "var(--accent)" : "var(--text-4)"}}/>
              <span style={{fontWeight: 600, letterSpacing:".05em"}}>{s.label.toUpperCase()}</span>
              <span style={cpS.sectRange}>{s.range}</span>
            </button>
          ))}
          <button style={cpS.addSect}><Ic k="plus" size={11}/> Section</button>
        </div>
        <div style={{display:"flex",alignItems:"center",gap:6}}>
          <button style={cpS.tool}>Edit section</button>
          <button style={cpS.tool}>Export chords</button>
        </div>
      </div>

      <div style={cpS.body}>
        <div style={cpS.workspace}>

          {/* Scale stripe */}
          <div style={cpS.scaleBar}>
            <div style={{display:"flex",alignItems:"baseline",gap:10}}>
              <span style={{fontSize:10.5, letterSpacing:".18em", color:"var(--text-3)"}}>SCALE</span>
              <span style={{fontSize: 22, fontFamily:"'Instrument Serif',serif", fontStyle:"italic", color:"var(--accent-soft)"}}>B minor</span>
              <span style={{fontSize:11, color:"var(--text-3)"}}>natural · 7 notes</span>
            </div>
            <div style={{display:"flex", gap: 3}}>
              {["B","C♯","D","E","F♯","G","A"].map((n,i)=>(
                <div key={i} style={{
                  padding:"4px 10px",
                  background: i===0 ? "var(--accent-dim)" : "var(--bg-2)",
                  color: i===0 ? "var(--accent-soft)" : "var(--text-2)",
                  border: `1px solid ${i===0 ? "var(--accent-line)" : "var(--line-2)"}`,
                  borderRadius: 5,
                  fontSize: 11,
                  fontFamily: "'Geist Mono',monospace"
                }}>{n}</div>
              ))}
            </div>
          </div>

          {/* Chord lane */}
          <div style={cpS.lane}>
            <div style={cpS.laneSide}>
              <span style={cpS.laneTitle}>Chords</span>
              <span style={cpS.laneMeta}>Post-Rock · 5 chords</span>
            </div>
            <div style={cpS.chordRow}>
              {COMPOSE_CHORDS.map((c, i) => (
                <ChordCard key={i} c={c} idx={i} />
              ))}
            </div>
          </div>

          {/* Instrument lanes */}
          {[
            { name: "Synth Bass", desc: "Resonance Wave", motif: "low" },
            { name: "Synth Pad", desc: "Resonance Wave", motif: "mid" },
            { name: "Lead Synth", desc: "Resonance Wave", motif: "high" },
          ].map((t, i) => (
            <div key={i} style={cpS.lane}>
              <div style={cpS.laneSide}>
                <span style={cpS.laneTitle}>{t.name}</span>
                <span style={cpS.laneMeta}>{t.desc}</span>
              </div>
              <div style={cpS.pianoRow}>
                <PianoMini variant={t.motif} />
              </div>
            </div>
          ))}

          {/* Drum grid */}
          <div style={cpS.drumLane}>
            <div style={cpS.laneSide}>
              <span style={cpS.laneTitle}>Drums</span>
              <span style={cpS.laneMeta}>16 steps · 6/8</span>
            </div>
            <div style={cpS.drumGrid}>
              <div style={cpS.drumHeader}>
                {Array.from({length:16}).map((_,i)=>(
                  <div key={i} style={{ ...cpS.stepHead, ...(i%4===0 ? cpS.stepHeadAcc : {}) }}>{i%4===0 ? (i/4|0)+1 : ""}</div>
                ))}
              </div>
              {DRUM_KIT.map((d, i) => (
                <div key={i} style={cpS.drumRow}>
                  <span style={cpS.drumName}>{d.name}</span>
                  <div style={cpS.drumCells}>
                    {d.pattern.map((on, j) => (
                      <div key={j} style={{
                        ...cpS.cell,
                        ...(j%4===0 ? cpS.cellAcc : {}),
                        ...(on ? { background: d.color, borderColor: d.color, opacity: .92 } : {})
                      }}/>
                    ))}
                  </div>
                </div>
              ))}
            </div>
          </div>

        </div>

        {/* Right rail */}
        <aside style={cpS.rail}>
          <div style={cpS.railGroup}>
            <div style={cpS.railTitle}>Chord generator</div>
            <Field label="Style">
              <Select value="Post-Rock" />
            </Field>
            <div style={{display:"grid",gridTemplateColumns:"1fr 1fr",gap:10}}>
              <Field label="Chords"><Stepper value={5} /></Field>
              <Field label="Beats / chord"><Stepper value={4} /></Field>
            </div>
            <div style={{display:"flex", gap:10}}>
              <Field label="Start °"><Select value="(any)" /></Field>
              <Field label="End °"><Select value="(any)" /></Field>
            </div>
            <Toggle label="Seventh chords" on={false}/>
            <div style={{display:"flex", gap: 8, marginTop: 4}}>
              <button style={cpS.primary}>Generate</button>
              <button style={cpS.secondary}>↻</button>
            </div>
            <div style={{ fontSize: 10, color: "var(--text-3)", fontFamily: "'Geist Mono',monospace", paddingTop: 4 }}>
              seed · 0×A50115C9A22B6F2F
            </div>
          </div>

          <div style={cpS.railGroup}>
            <div style={cpS.railTitle}>Section motif</div>
            <Field label="Source"><Select value="Manual"/></Field>
            <Field label="Complexity">
              <Slider value={0.35}/>
            </Field>
            <div style={{padding:"10px 12px", background:"var(--bg-2)", borderRadius: 8, border:"1px solid var(--line-2)"}}>
              <div style={{display:"flex", justifyContent:"space-between"}}>
                <span style={{fontSize:10.5, color:"var(--text-3)", letterSpacing:".14em"}}>MOTIF</span>
                <span style={{fontSize:10.5, color:"var(--text-3)", fontFamily:"'Geist Mono',monospace"}}>9 notes</span>
              </div>
              <MotifMini />
            </div>
            <div style={{ fontSize: 10.5, color: "var(--text-3)", lineHeight: 1.4 }}>
              Click a cell to add a note. Right-click to toggle accent. Scroll a note to cycle duration.
            </div>
          </div>
        </aside>
      </div>
    </div>
  );
}

function ChordCard({ c, idx }) {
  const playing = idx === 4;
  if (c.empty) {
    return <div style={{ ...cpS.chord, ...cpS.chordEmpty }}><Ic k="plus" size={13}/></div>;
  }
  return (
    <div style={{
      ...cpS.chord,
      ...(playing ? cpS.chordOn : {}),
      ...(c.locked ? cpS.chordLocked : {}),
    }}>
      <div style={{ display: "flex", justifyContent: "space-between", alignItems: "flex-start" }}>
        <span style={cpS.chordDeg}>{c.deg}</span>
        {c.locked && <span style={cpS.chordLock}>●</span>}
      </div>
      <div style={cpS.chordSym}>{c.sym}</div>
      <div style={cpS.chordBars}>{c.bars} bar</div>
    </div>
  );
}

function PianoMini({ variant }) {
  const rows = 12;
  const steps = 32;
  const seedFor = { low: 5, mid: 11, high: 19 }[variant] || 7;
  return (
    <div style={{ position: "relative", height: "100%", padding: "8px 6px", display: "grid", gridTemplateRows: "1fr 1fr 1fr 1fr 1fr", gap: 1, background: "var(--bg-2)", borderRadius: 8, border: "1px solid var(--line-2)" }}>
      {/* faint rows */}
      {Array.from({length:5}).map((_,r)=>(
        <div key={r} style={{ background: r%2 ? "rgba(255,255,255,.005)" : "transparent", borderBottom: r<4 ? "1px dashed var(--line-2)" : "none", position: "relative" }}>
          {Array.from({length:steps}).map((_,c) => {
            const trig = ((c*7 + r*13 + seedFor) % 9) < 3 && r !== (variant==="low"?0:variant==="high"?4:2);
            const lead = r === (variant==="low"?3:variant==="high"?1:2);
            if (!lead && !trig) return null;
            const present = lead ? ((c+seedFor)%5)<3 : trig;
            if (!present) return null;
            return <div key={c} style={{
              position:"absolute",
              top: 3, bottom: 3,
              left: `calc(${(c/steps)*100}% + 2px)`,
              width: `calc(${(1/steps)*100}% - 4px)`,
              background: "var(--accent)",
              opacity: .85,
              borderRadius: 2
            }}/>;
          })}
        </div>
      ))}
      {/* bar lines */}
      {Array.from({length: 8}).map((_, b) => (
        <div key={b} style={{ position: "absolute", left: `calc(${(b/8)*100}% + 6px)`, top: 4, bottom: 4, width: 1, background: "var(--line-2)", opacity: .6 }}/>
      ))}
    </div>
  );
}

function MotifMini() {
  const notes = [0,2,1,0,3,2,4,3,1];
  return (
    <div style={{ position: "relative", height: 60, marginTop: 8 }}>
      {notes.map((n,i)=>(
        <div key={i} style={{
          position: "absolute",
          left: `${(i/9)*100}%`,
          width: `${(1/9)*100 - 2}%`,
          top: `${n*12}px`,
          height: 6,
          background: "var(--accent-soft)",
          borderRadius: 2
        }}/>
      ))}
    </div>
  );
}

function Field({ label, children }) {
  return (
    <div style={{display:"flex",flexDirection:"column",gap:5}}>
      <span style={{fontSize:10, letterSpacing:".14em", color:"var(--text-3)"}}>{label.toUpperCase()}</span>
      {children}
    </div>
  );
}
function Select({ value }) {
  return (
    <button style={{
      display:"flex", justifyContent:"space-between", alignItems:"center",
      padding:"7px 10px", background:"var(--bg-2)", border:"1px solid var(--line)",
      borderRadius: 7, color:"var(--text-1)", fontSize: 12, cursor:"pointer", textAlign:"left"
    }}>
      <span>{value}</span>
      <Ic k="chevron" size={13}/>
    </button>
  );
}
function Stepper({ value }) {
  return (
    <div style={{
      display:"flex", alignItems:"center", justifyContent:"space-between",
      padding:"4px 6px 4px 12px", background:"var(--bg-2)", border:"1px solid var(--line)",
      borderRadius: 7, color:"var(--text-1)", fontSize: 12
    }}>
      <span style={{ fontFamily: "'Geist Mono',monospace" }}>{value}</span>
      <div style={{ display: "flex", flexDirection: "column" }}>
        <button style={{ width: 18, height: 13, padding: 0, background: "transparent", border: "none", color: "var(--text-3)", cursor: "pointer", fontSize: 9 }}>▲</button>
        <button style={{ width: 18, height: 13, padding: 0, background: "transparent", border: "none", color: "var(--text-3)", cursor: "pointer", fontSize: 9 }}>▼</button>
      </div>
    </div>
  );
}
function Toggle({ label, on }) {
  return (
    <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer" }}>
      <span style={{
        width: 30, height: 16, background: on ? "var(--accent)" : "var(--bg-3)",
        borderRadius: 999, position: "relative", border: "1px solid var(--line)", transition: "background .15s"
      }}>
        <span style={{
          position: "absolute", top: 1, left: on ? 14 : 1,
          width: 12, height: 12, background: "white", borderRadius: 999, transition: "left .15s"
        }}/>
      </span>
      <span style={{ fontSize: 12, color: "var(--text-1)" }}>{label}</span>
    </label>
  );
}
function Slider({ value }) {
  return (
    <div style={{ position: "relative", height: 20, display: "flex", alignItems: "center" }}>
      <div style={{ height: 3, width: "100%", background: "var(--bg-3)", borderRadius: 2, position: "relative" }}>
        <div style={{ position: "absolute", left: 0, height: "100%", width: `${value*100}%`, background: "var(--accent)", borderRadius: 2 }}/>
        <div style={{ position: "absolute", left: `calc(${value*100}% - 6px)`, top: -4.5, width: 12, height: 12, background: "white", border: "2px solid var(--accent)", borderRadius: 999 }}/>
      </div>
    </div>
  );
}

const cpS = {
  root: { display: "flex", flexDirection: "column", height: "100%", background: "var(--bg-1)" },
  toolbar: { display: "flex", justifyContent: "space-between", alignItems: "center", padding: "10px 16px", borderBottom: "1px solid var(--line-2)", height: 44 },
  sectTab: { display: "inline-flex", alignItems: "center", gap: 8, padding: "5px 12px", background: "transparent", border: "1px solid var(--line-2)", color: "var(--text-3)", borderRadius: 6, cursor: "pointer", fontSize: 11 },
  sectTabOn: { background: "var(--bg-2)", color: "var(--text-1)", borderColor: "var(--line)" },
  sectDot: { width: 7, height: 7, borderRadius: 999 },
  sectRange: { fontFamily: "'Geist Mono',monospace", fontSize: 10.5, color: "var(--text-3)" },
  addSect: { display: "inline-flex", alignItems: "center", gap: 5, padding: "5px 11px", border: "1px dashed var(--line)", background: "transparent", color: "var(--text-3)", borderRadius: 6, fontSize: 11, cursor: "pointer" },
  tool: { display: "inline-flex", alignItems: "center", gap: 6, padding: "5px 11px", background: "transparent", border: "1px solid transparent", color: "var(--text-2)", borderRadius: 6, fontSize: 12, cursor: "pointer" },

  body: { flex: 1, display: "grid", gridTemplateColumns: "1fr 280px", overflow: "hidden", minHeight: 0 },
  workspace: { padding: 16, overflowY: "auto", display: "flex", flexDirection: "column", gap: 12 },
  scaleBar: { display: "flex", alignItems: "center", justifyContent: "space-between", padding: "10px 16px", background: "var(--bg-2)", border: "1px solid var(--line-2)", borderRadius: 12 },

  lane: { display: "grid", gridTemplateColumns: "150px 1fr", gap: 12, alignItems: "stretch" },
  laneSide: { padding: "12px 4px", display: "flex", flexDirection: "column", justifyContent: "center" },
  laneTitle: { fontSize: 13, color: "var(--text-1)", fontWeight: 500 },
  laneMeta: { fontSize: 11, color: "var(--text-3)" },

  chordRow: { display: "grid", gridTemplateColumns: "repeat(8, 1fr)", gap: 6 },
  chord: { padding: "10px 12px", background: "var(--bg-2)", border: "1px solid var(--line-2)", borderRadius: 9, display: "flex", flexDirection: "column", justifyContent: "space-between", minHeight: 64, cursor: "pointer", transition: "all .15s" },
  chordOn: { background: "var(--accent-dim)", borderColor: "var(--accent-line)", boxShadow: "0 0 0 3px rgba(139,109,255,.10)" },
  chordLocked: { borderStyle: "solid" },
  chordEmpty: { background: "transparent", borderStyle: "dashed", color: "var(--text-3)", display: "grid", placeItems: "center", minHeight: 64 },
  chordDeg: { fontSize: 10, color: "var(--text-3)", letterSpacing: ".14em", fontFamily: "'Geist Mono',monospace", textTransform: "uppercase" },
  chordLock: { fontSize: 7, color: "var(--accent-soft)" },
  chordSym: { fontSize: 22, fontFamily: "'Instrument Serif',serif", fontStyle: "italic", color: "var(--text-1)", lineHeight: 1 },
  chordBars: { fontSize: 9.5, color: "var(--text-3)", letterSpacing: ".1em" },

  pianoRow: { height: 64 },

  drumLane: { display: "grid", gridTemplateColumns: "150px 1fr", gap: 12, marginTop: 4 },
  drumGrid: { background: "var(--bg-2)", border: "1px solid var(--line-2)", borderRadius: 12, padding: 8 },
  drumHeader: { display: "grid", gridTemplateColumns: "60px repeat(16, 1fr)", paddingLeft: 60, gap: 2, marginBottom: 6, height: 12 },
  stepHead: { fontSize: 9, color: "var(--text-4)", textAlign: "center", fontFamily: "'Geist Mono',monospace" },
  stepHeadAcc: { color: "var(--text-3)" },
  drumRow: { display: "grid", gridTemplateColumns: "60px 1fr", alignItems: "center", gap: 6, marginBottom: 3 },
  drumName: { fontSize: 11, color: "var(--text-2)" },
  drumCells: { display: "grid", gridTemplateColumns: "repeat(16, 1fr)", gap: 2 },
  cell: { height: 18, background: "var(--bg-1)", border: "1px solid var(--line-2)", borderRadius: 3, transition: "background .1s" },
  cellAcc: { borderColor: "var(--line)" },

  rail: { borderLeft: "1px solid var(--line-2)", padding: 18, display: "flex", flexDirection: "column", gap: 18, overflow: "auto", background: "var(--bg-1)" },
  railGroup: { display: "flex", flexDirection: "column", gap: 10 },
  railTitle: { fontSize: 13, color: "var(--text-1)", fontWeight: 500, paddingBottom: 6, borderBottom: "1px solid var(--line-2)" },
  primary: { flex: 1, padding: "8px 14px", background: "var(--accent)", color: "#0e0a1f", border: "none", borderRadius: 7, fontSize: 12, fontWeight: 600, cursor: "pointer" },
  secondary: { width: 36, padding: "8px", background: "var(--bg-2)", color: "var(--text-1)", border: "1px solid var(--line)", borderRadius: 7, fontSize: 12, cursor: "pointer" },
};

window.Compose = Compose;
