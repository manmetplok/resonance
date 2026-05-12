/* Mixer view — channel strips reimagined */

const MIX_TRACKS = [
  { name: "Drums", out: "Bus 1", level: 0.78, pan: 0, instr: "Resonance Drums", inserts: ["Comp", "EQ-8"], color: "drums" },
  { name: "Synth Bass", out: "Master", level: 0.62, pan: -0.15, instr: "+ Instrument", inserts: ["Sat"], color: "bass" },
  { name: "Synth Pad", out: "Master", level: 0.42, pan: 0.20, instr: "Resonance Wave", inserts: ["Verb", "Chorus"], color: "pad" },
  { name: "Lead Synth", out: "Master", level: 0.55, pan: 0, instr: "+ Instrument", inserts: [], color: "lead" },
  { name: "Vox FX", out: "Bus 2", level: 0.30, pan: 0.40, instr: "+ Instrument", inserts: ["Delay"], color: "fx" },
];

function Mixer({ accent }) {
  const [selected, setSelected] = React.useState(0);
  return (
    <div style={mxS.root}>
      <div style={mxS.toolbar}>
        <div style={{display:"flex",alignItems:"center",gap:14}}>
          <span style={mxS.tbLabel}>MIX</span>
          <div style={mxS.segmented}>
            {["Tracks","Buses","FX"].map((s,i)=>(
              <button key={s} style={{ ...mxS.seg, ...(i===0?mxS.segOn:{}) }}>{s}</button>
            ))}
          </div>
        </div>
        <div style={{display:"flex",alignItems:"center",gap:6}}>
          <button style={mxS.tool}>Snapshot</button>
          <button style={mxS.tool}>Link</button>
          <button style={mxS.tool}><Ic k="sliders" size={13}/> Layout</button>
        </div>
      </div>

      <div style={mxS.body}>
        <div style={mxS.stripsScroll}>
          {MIX_TRACKS.map((t, i) => (
            <Strip key={i} t={t} selected={selected===i} onSelect={()=>setSelected(i)} accent={accent} />
          ))}
          <div style={mxS.busDivider}>
            <span style={mxS.busLabel}>BUSES</span>
          </div>
          <BusStrip name="Bus 1 · Drums" tracks="1 send" inserts={["Comp", "IR"]} level={0.7} />
          <BusStrip name="Bus 2 · FX" tracks="1 send" inserts={["Verb"]} level={0.55} />
          <button style={mxS.addBus}><Ic k="plus" size={13}/> Add bus</button>
          <div style={mxS.spacer}/>
          <MasterStrip />
        </div>

        <Inspector track={MIX_TRACKS[selected]} accent={accent} />
      </div>
    </div>
  );
}

function Strip({ t, selected, onSelect, accent }) {
  const dB = ((t.level - 0.75) * 60).toFixed(1);
  return (
    <div onClick={onSelect} style={{ ...mxS.strip, ...(selected ? mxS.stripSel : {}) }}>
      <div style={mxS.stripHead}>
        <span style={{ ...mxS.stripIcon, color: "var(--accent-soft)" }}>{glyphFor(t.color)}</span>
        <span style={mxS.stripName}>{t.name}</span>
      </div>

      <div style={mxS.stripCtrls}>
        <button style={{ ...mxS.miniBtn, ...(t.muted ? mxS.miniMute : {}) }}>M</button>
        <button style={mxS.miniBtn}>S</button>
        <button style={mxS.miniBtn}><Ic k="rec" size={8} /></button>
        <button style={mxS.miniBtn}><Ic k="headphones" size={10}/></button>
      </div>

      <div style={mxS.instrSlot}>
        <span style={mxS.slotIcon}>◆</span>
        <span style={mxS.slotName}>{t.instr}</span>
      </div>

      <div style={mxS.insertList}>
        {t.inserts.map((x,i) => (
          <div key={i} style={mxS.insert}>{x}</div>
        ))}
        <button style={mxS.insertAdd}><Ic k="plus" size={10}/> FX</button>
      </div>

      <div style={mxS.knobRow}>
        <Knob label="PAN" value={t.pan} />
        <Knob label="SEND" value={0.2} />
      </div>

      <div style={mxS.faderArea}>
        <FaderTrack level={t.level} />
        <Meterv level={t.level} />
      </div>

      <div style={mxS.dbLabel}>
        <span style={{ fontFamily: "'Geist Mono',monospace", fontSize: 11, color: "var(--text-1)" }}>{dB > 0 ? "+" : ""}{dB}</span>
        <span style={{ fontSize: 9, color: "var(--text-3)", letterSpacing: ".1em" }}>dB</span>
      </div>

      <div style={mxS.routing}>
        <span style={mxS.routeLabel}>OUT</span>
        <span style={mxS.routeVal}>→ {t.out}</span>
      </div>
    </div>
  );
}

function BusStrip({ name, tracks, inserts, level }) {
  return (
    <div style={{ ...mxS.strip, ...mxS.bus }}>
      <div style={mxS.stripHead}>
        <span style={{ ...mxS.stripIcon, background: "var(--bg-3)", color: "var(--warm)" }}>◌</span>
        <span style={{ ...mxS.stripName, color: "var(--warm)" }}>{name}</span>
      </div>
      <div style={{ fontSize: 10, color: "var(--text-3)", padding: "0 14px" }}>{tracks}</div>
      <div style={{height:8}}/>
      <div style={mxS.insertList}>
        {inserts.map((x,i)=>(<div key={i} style={mxS.insert}>{x}</div>))}
        <button style={mxS.insertAdd}><Ic k="plus" size={10}/> FX</button>
      </div>
      <div style={{height:8}}/>
      <div style={mxS.faderArea}>
        <FaderTrack level={level} warm />
        <Meterv level={level} />
      </div>
      <div style={mxS.dbLabel}>
        <span style={{ fontFamily: "'Geist Mono',monospace", fontSize: 11, color: "var(--text-1)" }}>{((level-0.75)*60).toFixed(1)}</span>
        <span style={{ fontSize: 9, color: "var(--text-3)" }}>dB</span>
      </div>
    </div>
  );
}

function MasterStrip() {
  return (
    <div style={{ ...mxS.strip, ...mxS.master }}>
      <div style={{ ...mxS.stripHead, justifyContent: "center" }}>
        <span style={{ fontSize: 11, color: "var(--text-1)", letterSpacing: ".18em", fontWeight: 600 }}>MASTER</span>
      </div>
      <div style={{height: 10}}/>
      <div style={mxS.insertList}>
        <div style={mxS.insert}>Limiter</div>
        <div style={mxS.insert}>Tape</div>
      </div>
      <div style={{flex:1}}/>
      <div style={{display:"grid", gridTemplateColumns:"1fr 1fr", gap: 8, padding:"0 12px"}}>
        <div style={{textAlign:"center"}}>
          <div style={{ fontSize: 18, fontFamily:"'Geist Mono',monospace", color: "var(--text-1)" }}>−2.1</div>
          <div style={{ fontSize: 9, color: "var(--text-3)", letterSpacing:".14em" }}>OUT dB</div>
        </div>
        <div style={{textAlign:"center"}}>
          <div style={{ fontSize: 18, fontFamily:"'Geist Mono',monospace", color: "var(--good)" }}>−0.0</div>
          <div style={{ fontSize: 9, color: "var(--text-3)", letterSpacing:".14em" }}>PEAK</div>
        </div>
      </div>
      <div style={mxS.faderArea}>
        <FaderTrack level={0.74} warm />
        <Meterv level={0.74} tall />
      </div>
      <div style={{ display:"flex",justifyContent:"center", gap: 6, padding: "0 14px 4px" }}>
        <button style={mxS.tinyTag}>BOUNCE</button>
      </div>
    </div>
  );
}

function Knob({ label, value }) {
  // value: -1 to 1
  const angle = value * 130;
  return (
    <div style={{ display:"flex", flexDirection:"column", alignItems:"center", gap: 4 }}>
      <div style={{ width: 28, height: 28, borderRadius: "50%", background: "var(--bg-3)", border: "1px solid var(--line)", position: "relative", boxShadow: "inset 0 1px 0 rgba(255,255,255,.04)" }}>
        <div style={{ position:"absolute", top: 3, left: "50%", width: 1.5, height: 8, background: "var(--accent-soft)", transform: `translateX(-50%) rotate(${angle}deg)`, transformOrigin: "50% 11px" }}/>
      </div>
      <span style={{ fontSize: 8.5, color: "var(--text-3)", letterSpacing: ".12em" }}>{label}</span>
    </div>
  );
}

function FaderTrack({ level, warm }) {
  return (
    <div style={{ position: "relative", width: 22, height: "100%", background: "var(--bg-1)", borderRadius: 3, border: "1px solid var(--line-2)" }}>
      {/* tick marks */}
      {[0, .25, .5, .75, 1].map((v) => (
        <div key={v} style={{ position: "absolute", left: -6, right: -6, top: `${(1-v)*100}%`, height: 1, background: "var(--line)" }}/>
      ))}
      {/* fader cap */}
      <div style={{
        position:"absolute", left: -3, right: -3, top: `calc(${(1-level)*100}% - 6px)`,
        height: 14, background: "linear-gradient(180deg, var(--bg-4), var(--bg-3))",
        border: `1px solid ${warm ? "rgba(232,196,123,.5)" : "var(--accent-line)"}`,
        borderRadius: 3, boxShadow: "0 1px 3px rgba(0,0,0,.4)",
      }}>
        <div style={{ position:"absolute", left: 3, right: 3, top: "50%", height: 1, background: warm ? "var(--warm)" : "var(--accent)" }}/>
      </div>
    </div>
  );
}

function Meterv({ level, tall }) {
  return (
    <div style={{ display:"flex", gap: 2, height: "100%" }}>
      {[level*0.95, level*0.85].map((v, i) => (
        <div key={i} style={{ width: 4, background: "var(--bg-1)", borderRadius: 2, position: "relative", overflow: "hidden", border: "1px solid var(--line-2)" }}>
          <div style={{
            position: "absolute", left: 0, right: 0, bottom: 0,
            height: `${v*100}%`,
            background: "linear-gradient(0deg, var(--good) 0%, var(--good) 60%, var(--warm) 80%, var(--bad) 100%)"
          }}/>
        </div>
      ))}
    </div>
  );
}

function Inspector({ track, accent }) {
  return (
    <aside style={mxS.inspector}>
      <div style={{display:"flex",alignItems:"center",justifyContent:"space-between"}}>
        <div>
          <div style={{ fontSize: 10.5, color: "var(--text-3)", letterSpacing: ".14em", textTransform:"uppercase" }}>Inspector</div>
          <div style={{ fontSize: 17, color: "var(--text-1)", marginTop: 2, fontWeight: 500 }}>{track.name}</div>
        </div>
        <button style={{...mxS.tool, padding: "5px 8px"}}><Ic k="more" size={14}/></button>
      </div>

      <div style={mxS.inspGroup}>
        <div style={mxS.inspGroupTitle}>SIGNAL</div>
        <div style={{display:"grid",gridTemplateColumns:"1fr 1fr",gap:10}}>
          <InspStat label="Peak" value="−6.2 dB" />
          <InspStat label="RMS" value="−18.4 dB" />
          <InspStat label="Pan" value={track.pan === 0 ? "C" : `${track.pan>0?"R":"L"} ${Math.abs(track.pan*100)|0}`} />
          <InspStat label="Out" value={track.out} />
        </div>
      </div>

      <div style={mxS.inspGroup}>
        <div style={mxS.inspGroupTitle}>ROUTING</div>
        <Row label="Input" value="—" />
        <Row label="Output" value={`→ ${track.out}`} accent />
        <Row label="Send A" value="Bus 1 · −12 dB" />
        <Row label="Send B" value="(none)" muted />
      </div>

      <div style={mxS.inspGroup}>
        <div style={mxS.inspGroupTitle}>CHAIN</div>
        {(track.inserts.length ? track.inserts : ["—"]).map((x,i)=>(
          <div key={i} style={mxS.chainRow}>
            <span style={{display:"inline-flex",alignItems:"center",gap:8}}>
              <span style={{width:6,height:6,borderRadius:999,background:"var(--accent)"}}/>
              <span style={{fontSize:12, color:"var(--text-1)"}}>{x}</span>
            </span>
            <span style={{fontSize:10, color:"var(--text-3)"}}>BYP</span>
          </div>
        ))}
        <button style={mxS.addInsert}><Ic k="plus" size={11}/> Add to chain</button>
      </div>
    </aside>
  );
}
function InspStat({label, value}) {
  return (
    <div style={{padding:"8px 10px", background:"var(--bg-1)", border:"1px solid var(--line-2)", borderRadius: 7}}>
      <div style={{fontSize:9.5, color:"var(--text-3)", letterSpacing:".14em"}}>{label.toUpperCase()}</div>
      <div style={{fontSize:13, fontFamily:"'Geist Mono',monospace", color:"var(--text-1)", marginTop:2}}>{value}</div>
    </div>
  );
}
function Row({label, value, accent, muted}) {
  return (
    <div style={{display:"flex",justifyContent:"space-between",padding:"6px 0",borderBottom:"1px dashed var(--line-2)"}}>
      <span style={{fontSize:11.5, color:"var(--text-3)"}}>{label}</span>
      <span style={{fontSize:12, color: accent ? "var(--accent-soft)" : muted ? "var(--text-4)" : "var(--text-1)", fontFamily: "'Geist Mono',monospace"}}>{value}</span>
    </div>
  );
}

const mxS = {
  root: { display: "flex", flexDirection: "column", height: "100%", background: "var(--bg-1)" },
  toolbar: { display: "flex", justifyContent: "space-between", alignItems: "center", padding: "10px 16px", borderBottom: "1px solid var(--line-2)", height: 44 },
  tbLabel: { fontSize: 10.5, letterSpacing: ".18em", color: "var(--text-3)", fontWeight: 600 },
  segmented: { display: "flex", background: "var(--bg-2)", padding: 3, borderRadius: 7, border: "1px solid var(--line-2)" },
  seg: { padding: "5px 12px", border: "none", background: "transparent", color: "var(--text-3)", fontSize: 11.5, borderRadius: 5, cursor: "pointer" },
  segOn: { background: "var(--bg-3)", color: "var(--text-1)" },
  tool: { display: "inline-flex", alignItems: "center", gap: 6, padding: "5px 10px", background: "transparent", border: "1px solid transparent", color: "var(--text-2)", borderRadius: 6, fontSize: 12, cursor: "pointer" },
  body: { flex: 1, display: "grid", gridTemplateColumns: "1fr 320px", overflow: "hidden" },
  stripsScroll: { display: "flex", padding: 16, gap: 10, overflowX: "auto", alignItems: "stretch" },
  strip: { width: 132, minWidth: 132, background: "var(--bg-2)", border: "1px solid var(--line-2)", borderRadius: 12, display: "flex", flexDirection: "column", padding: "10px 0", cursor: "pointer", transition: "border-color .15s" },
  stripSel: { borderColor: "var(--accent-line)", boxShadow: "0 0 0 3px var(--accent-dim)" },
  bus: { background: "rgba(232,196,123,.04)", borderColor: "rgba(232,196,123,.16)" },
  master: { background: "linear-gradient(180deg, var(--bg-2), rgba(139,109,255,.06))", borderColor: "var(--accent-line)", width: 156 },
  stripHead: { display: "flex", alignItems: "center", gap: 8, padding: "0 12px 8px", borderBottom: "1px solid var(--line-2)" },
  stripIcon: { width: 22, height: 22, display: "grid", placeItems: "center", background: "var(--bg-3)", borderRadius: 5 },
  stripName: { fontSize: 12, color: "var(--text-1)", fontWeight: 500, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" },
  stripCtrls: { display: "grid", gridTemplateColumns: "1fr 1fr 1fr 1fr", gap: 3, padding: "8px 12px 0" },
  miniBtn: { height: 20, display: "grid", placeItems: "center", background: "var(--bg-1)", border: "1px solid var(--line-2)", color: "var(--text-3)", fontSize: 9.5, fontWeight: 600, fontFamily: "'Geist Mono',monospace", borderRadius: 4, cursor: "pointer" },
  miniMute: { color: "var(--bad)", borderColor: "rgba(232,123,139,.4)" },
  instrSlot: { display: "flex", alignItems: "center", gap: 6, margin: "10px 12px 6px", padding: "6px 8px", background: "rgba(139,109,255,.10)", border: "1px solid var(--accent-line)", borderRadius: 6 },
  slotIcon: { fontSize: 8, color: "var(--accent-soft)" },
  slotName: { fontSize: 10.5, color: "var(--accent-soft)", whiteSpace: "nowrap", overflow:"hidden", textOverflow: "ellipsis" },
  insertList: { display: "flex", flexDirection: "column", gap: 3, padding: "0 12px", margin: "0 0 8px" },
  insert: { fontSize: 10.5, color: "var(--text-2)", padding: "4px 8px", background: "var(--bg-1)", border: "1px solid var(--line-2)", borderRadius: 5 },
  insertAdd: { fontSize: 10, color: "var(--text-3)", padding: "4px 8px", background: "transparent", border: "1px dashed var(--line)", borderRadius: 5, cursor: "pointer", display: "inline-flex", alignItems: "center", gap: 4, justifyContent: "center" },
  knobRow: { display: "flex", justifyContent: "space-around", padding: "0 12px 8px" },
  faderArea: { flex: 1, minHeight: 140, display: "flex", justifyContent: "center", gap: 8, padding: "8px 14px", alignItems: "stretch" },
  dbLabel: { display: "flex", flexDirection: "column", alignItems: "center", padding: "4px 0", borderTop: "1px solid var(--line-2)", marginTop: 4 },
  routing: { display: "flex", justifyContent: "space-between", padding: "6px 12px 0", borderTop: "1px dashed var(--line-2)", marginTop: 4 },
  routeLabel: { fontSize: 9, color: "var(--text-3)", letterSpacing: ".14em" },
  routeVal: { fontSize: 10, color: "var(--text-1)", fontFamily: "'Geist Mono',monospace" },
  busDivider: { display: "flex", alignItems: "center", justifyContent: "center", borderLeft: "1px dashed var(--line)", margin: "0 8px" },
  busLabel: { fontSize: 9, letterSpacing: ".2em", color: "var(--text-3)", writingMode: "vertical-rl", transform: "rotate(180deg)" },
  addBus: { width: 44, alignSelf: "stretch", border: "1px dashed var(--line)", background: "transparent", color: "var(--text-3)", borderRadius: 12, display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center", gap: 6, cursor: "pointer", fontSize: 10 },
  spacer: { flex: 1 },
  tinyTag: { fontSize: 9, letterSpacing: ".18em", padding: "3px 9px", border: "1px solid var(--accent-line)", background: "var(--accent-dim)", color: "var(--accent-soft)", borderRadius: 4, cursor: "pointer" },

  inspector: { borderLeft: "1px solid var(--line-2)", padding: 18, overflow: "auto", display: "flex", flexDirection: "column", gap: 16, background: "var(--bg-1)" },
  inspGroup: { display: "flex", flexDirection: "column", gap: 8 },
  inspGroupTitle: { fontSize: 10, letterSpacing: ".18em", color: "var(--text-3)", fontWeight: 600, paddingBottom: 4, borderBottom: "1px solid var(--line-2)" },
  chainRow: { display: "flex", justifyContent: "space-between", alignItems: "center", padding: "8px 10px", background: "var(--bg-2)", border: "1px solid var(--line-2)", borderRadius: 7 },
  addInsert: { fontSize: 11.5, color: "var(--text-3)", padding: "8px 10px", background: "transparent", border: "1px dashed var(--line)", borderRadius: 7, cursor: "pointer", display: "inline-flex", alignItems: "center", gap: 6, justifyContent: "center" },
};

window.Mixer = Mixer;
