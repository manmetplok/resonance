//! Seeded noise for the diffusion / rectified-flow models.
//!
//! Every DiffSinger acoustic, pitch and variance export draws its starting
//! noise inside the graph (`RandomNormalLike`, and on some exports a
//! `RandomUniformLike` too), and the NSF-HiFiGAN vocoders draw their
//! harmonic phases (`RandomUniform`) and excitation noise
//! (`RandomNormalLike`) the same way. ONNX Runtime seeds those ops from
//! the clock when the session is built and advances them on every run, so the same
//! input renders differently every time — and differently depending on
//! what the session rendered before. That is why two renders of one take
//! used to differ by ±10 Whisper points.
//!
//! [`seed_random_ops`] rewrites the model bytes before the session is
//! built: each top-level `Random*Like(x) -> y` becomes
//!
//! ```text
//! s = Shape(x); n = ReduceProd(s); y = Reshape(Slice(noise_k, [0], n), s)
//! ```
//!
//! (a `RandomNormal` / `RandomUniform` with a fixed `shape` the same, with
//! `s` and `n` constants), `noise_k` being a new float graph input; for a
//! `*Like` op `n` is also a graph output (`resonance_rn<k>_n`), so the
//! session can learn how many values the op draws for a given input
//! length and feed exactly that many ([`super::common::SeededSession`]).
//! The caller feeds the inputs from [`noise`], a counter-based generator
//! keyed by a seed it chooses (the render unit's content key), so a unit
//! renders bit-identically however often and in whatever order it is
//! rendered. The buffer only has to be at least as long as the tensor the
//! op would have drawn; the stream is prefix-stable, so its length never
//! changes the values.
//!
//! The rewrite works on the protobuf wire format directly (no ONNX crate):
//! `ModelProto.graph` (field 7) → `GraphProto.node` (1) / `.input` (11) /
//! `.output` (12). Everything else is copied byte for byte.
//!
//! **What stays unseeded.** Only the main graph's own nodes are rewritten.
//! A random op inside a control-flow body (`If` / `Loop` / `Scan`
//! attribute graphs) or a model-local function would need a different
//! slice of noise per iteration, so it is left alone and *counted*
//! ([`SeededModel::unseeded`]): the model still loads, but it is not
//! repeatable, and the caller says so once. The same holds for a random op
//! whose output is not float (an explicit non-float `dtype`, or a `*Like`
//! over a non-float input), for a model older than opset 12 (the
//! replacement uses `Constant.value_ints`), and for a model whose weights
//! live in external data files (it has to be loaded from its path, so it
//! cannot be rewritten in memory). None of the shipped banks has any of
//! these.

use anyhow::{anyhow, bail, Result};

/// The distribution one replaced op drew from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoiseDist {
    Normal { mean: f32, scale: f32 },
    Uniform { low: f32, high: f32 },
}

/// How many values one replaced op draws per run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NoiseSize {
    /// A fixed `shape` (the vocoders' `RandomUniform([1, harmonics])`):
    /// exactly this many.
    Fixed(usize),
    /// As many as the `*Like` op's input has, which the run decides; the
    /// graph reports it in output [`NoiseSpec::count_output`].
    Like,
}

/// One noise input [`seed_random_ops`] added: its graph input name and
/// what to fill it with.
#[derive(Debug, Clone, PartialEq)]
pub struct NoiseSpec {
    pub input: String,
    pub dist: NoiseDist,
    pub size: NoiseSize,
    /// The graph output carrying how many values the op drew (`*Like`
    /// ops only).
    pub count_output: Option<String>,
}

/// Prefix of the graph inputs [`seed_random_ops`] adds.
pub const NOISE_INPUT_PREFIX: &str = "resonance_noise_";

/// What [`seed_random_ops`] made of a model.
#[derive(Debug, Clone, Default)]
pub struct SeededModel {
    /// The rewritten model, or `None` when nothing was rewritten (load
    /// the original — from its file, which also resolves external data).
    pub bytes: Option<Vec<u8>>,
    /// One per replaced op, in graph order.
    pub specs: Vec<NoiseSpec>,
    /// The random ops left drawing from the runtime's clock-seeded
    /// generator, each with why (see the module docs). Empty for a model
    /// that renders repeatably.
    pub unseeded: Vec<String>,
}

impl SeededModel {
    /// Whether every random op of the model reads fed noise.
    pub fn repeatable(&self) -> bool {
        self.unseeded.is_empty()
    }
}

/// Rewrite `model` (a serialized `ModelProto`) so its top-level
/// `RandomNormal(Like)` / `RandomUniform(Like)` ops read a fed noise input
/// instead of the runtime's clock-seeded generator. A model with no op to
/// rewrite comes back as `bytes: None` (nothing copied). An error means
/// the bytes are not a model this parser can walk (truncated, or not
/// protobuf); the caller loads the file unseeded.
pub fn seed_random_ops(model: &[u8]) -> Result<SeededModel> {
    let scan = scan_model(model)?;
    let mut unseeded = scan.nested;
    let rewritable: Vec<&Candidate> = scan
        .candidates
        .iter()
        .filter(|c| match &c.skip {
            Some(why) => {
                unseeded.push(why.clone());
                false
            }
            None => true,
        })
        .collect();
    let blocker = if scan.external_data {
        Some("its weights are in external data files")
    } else if scan.opset < 12 {
        Some("its opset is older than 12")
    } else {
        None
    };
    if let Some(why) = blocker {
        unseeded.extend(rewritable.iter().map(|c| format!("{} ({why})", c.op)));
        return Ok(SeededModel {
            bytes: None,
            specs: Vec::new(),
            unseeded,
        });
    }
    if rewritable.is_empty() {
        return Ok(SeededModel {
            bytes: None,
            specs: Vec::new(),
            unseeded,
        });
    }
    let mut out = Vec::with_capacity(model.len() + 4096);
    let mut specs = Vec::new();
    let mut pos = 0;
    while pos < model.len() {
        let field = read_field(model, &mut pos)?;
        if field.num == 7 && field.wire == WIRE_LEN {
            let graph = rewrite_graph(field.payload, &scan.candidates, &mut specs)?;
            put_len_field(&mut out, 7, &graph);
        } else {
            out.extend_from_slice(field.raw);
        }
    }
    Ok(SeededModel {
        bytes: Some(out),
        specs,
        unseeded,
    })
}

/// A top-level random node, found by [`scan_model`].
struct Candidate {
    /// Its index among the graph's nodes.
    index: usize,
    op: String,
    node: RandomNode,
    /// Why it cannot be rewritten, if it cannot.
    skip: Option<String>,
}

/// What a read-only pass over the model found.
struct ModelScan {
    /// The default-domain opset (0 when the model names none).
    opset: i64,
    external_data: bool,
    candidates: Vec<Candidate>,
    /// Random ops in subgraphs and functions.
    nested: Vec<String>,
}

const ONNX_FLOAT: i64 = 1;

fn scan_model(model: &[u8]) -> Result<ModelScan> {
    let mut scan = ModelScan {
        opset: 0,
        external_data: false,
        candidates: Vec::new(),
        nested: Vec::new(),
    };
    let mut graph: Option<&[u8]> = None;
    let mut pos = 0;
    while pos < model.len() {
        let field = read_field(model, &mut pos)?;
        match (field.num, field.wire) {
            (7, WIRE_LEN) => graph = Some(field.payload),
            (8, WIRE_LEN) => {
                let (domain, version) = parse_opset(field.payload)?;
                if domain.is_empty() || domain == "ai.onnx" {
                    scan.opset = version;
                }
            }
            // ModelProto.functions: random ops in them stay unseeded.
            (25, WIRE_LEN) => count_in_function(field.payload, &mut scan)?,
            _ => {}
        }
    }
    let graph = graph.ok_or_else(|| anyhow!("ONNX model without a graph"))?;
    // Element types of the graph's named values, for a `*Like` op
    // without `dtype` (its output takes its input's type).
    let mut dtypes: Vec<(String, i64)> = Vec::new();
    let mut nodes: Vec<&[u8]> = Vec::new();
    let mut pos = 0;
    while pos < graph.len() {
        let field = read_field(graph, &mut pos)?;
        match (field.num, field.wire) {
            (1, WIRE_LEN) => nodes.push(field.payload),
            (5, WIRE_LEN) => {
                let (name, dtype, external) = parse_initializer(field.payload)?;
                scan.external_data |= external;
                dtypes.push((name, dtype));
            }
            (11 | 13, WIRE_LEN) => {
                if let Some((name, dtype)) = parse_value_info(field.payload)? {
                    dtypes.push((name, dtype));
                }
            }
            _ => {}
        }
    }
    for (index, node) in nodes.iter().enumerate() {
        let parsed = parse_node(node)?;
        for sub in &parsed.subgraphs {
            count_in_graph(sub, &mut scan)?;
        }
        let Some(random) = parsed.random()? else {
            if is_random_op(&parsed.op) {
                scan.nested
                    .push(format!("{} (not a kind the rewrite handles)", parsed.op));
            }
            continue;
        };
        let dtype = parsed
            .attr("dtype")
            .and_then(|a| a.i)
            .or_else(|| match &random.shape {
                ShapeFrom::Like(input) => dtypes.iter().find(|(n, _)| n == input).map(|(_, d)| *d),
                ShapeFrom::Fixed(_) => None,
            });
        let skip = match dtype {
            Some(d) if d != ONNX_FLOAT => Some(format!("{} with element type {d}", parsed.op)),
            _ => None,
        };
        scan.candidates.push(Candidate {
            index,
            op: parsed.op.clone(),
            node: random,
            skip,
        });
    }
    Ok(scan)
}

/// Count random ops anywhere in `graph` (a subgraph: none is rewritten).
fn count_in_graph(graph: &[u8], scan: &mut ModelScan) -> Result<()> {
    let mut pos = 0;
    while pos < graph.len() {
        let field = read_field(graph, &mut pos)?;
        match (field.num, field.wire) {
            (1, WIRE_LEN) => count_in_node(field.payload, "a subgraph", scan)?,
            (5, WIRE_LEN) => scan.external_data |= parse_initializer(field.payload)?.2,
            _ => {}
        }
    }
    Ok(())
}

/// Count random ops in a `FunctionProto` (its nodes are field 7).
fn count_in_function(function: &[u8], scan: &mut ModelScan) -> Result<()> {
    let mut pos = 0;
    while pos < function.len() {
        let field = read_field(function, &mut pos)?;
        if field.num == 7 && field.wire == WIRE_LEN {
            count_in_node(field.payload, "a model-local function", scan)?;
        }
    }
    Ok(())
}

fn count_in_node(node: &[u8], place: &str, scan: &mut ModelScan) -> Result<()> {
    let parsed = parse_node(node)?;
    if is_random_op(&parsed.op) {
        scan.nested.push(format!("{} in {place}", parsed.op));
    }
    for sub in &parsed.subgraphs {
        count_in_graph(sub, scan)?;
    }
    Ok(())
}

fn is_random_op(op: &str) -> bool {
    matches!(
        op,
        "RandomNormalLike"
            | "RandomUniformLike"
            | "RandomNormal"
            | "RandomUniform"
            | "Multinomial"
            | "Bernoulli"
    )
}

/// `OperatorSetIdProto`: (domain, version).
fn parse_opset(bytes: &[u8]) -> Result<(String, i64)> {
    let (mut domain, mut version) = (String::new(), 0);
    let mut pos = 0;
    while pos < bytes.len() {
        let field = read_field(bytes, &mut pos)?;
        match (field.num, field.wire) {
            (1, WIRE_LEN) => domain = utf8(field.payload)?,
            (2, WIRE_VARINT) => version = field.varint as i64,
            _ => {}
        }
    }
    Ok((domain, version))
}

/// `TensorProto`: (name, data_type, stored externally).
fn parse_initializer(bytes: &[u8]) -> Result<(String, i64, bool)> {
    let (mut name, mut dtype, mut external) = (String::new(), 0, false);
    let mut pos = 0;
    while pos < bytes.len() {
        let field = read_field(bytes, &mut pos)?;
        match (field.num, field.wire) {
            (2, WIRE_VARINT) => dtype = field.varint as i64,
            (8, WIRE_LEN) => name = utf8(field.payload)?,
            (13, WIRE_LEN) => external = true,
            (14, WIRE_VARINT) => external |= field.varint == 1,
            _ => {}
        }
    }
    Ok((name, dtype, external))
}

/// `ValueInfoProto` of a tensor: (name, elem_type).
fn parse_value_info(bytes: &[u8]) -> Result<Option<(String, i64)>> {
    let mut name = String::new();
    let mut dtype = None;
    let mut pos = 0;
    while pos < bytes.len() {
        let field = read_field(bytes, &mut pos)?;
        match (field.num, field.wire) {
            (1, WIRE_LEN) => name = utf8(field.payload)?,
            // TypeProto.tensor_type (1) → elem_type (1).
            (2, WIRE_LEN) => {
                let mut p = 0;
                while p < field.payload.len() {
                    let ty = read_field(field.payload, &mut p)?;
                    if ty.num == 1 && ty.wire == WIRE_LEN {
                        let mut q = 0;
                        while q < ty.payload.len() {
                            let t = read_field(ty.payload, &mut q)?;
                            if t.num == 1 && t.wire == WIRE_VARINT {
                                dtype = Some(t.varint as i64);
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(dtype.map(|d| (name, d)))
}

fn rewrite_graph(
    graph: &[u8],
    candidates: &[Candidate],
    specs: &mut Vec<NoiseSpec>,
) -> Result<Vec<u8>> {
    let mut out = Vec::with_capacity(graph.len() + 4096);
    let mut pos = 0;
    let mut node_index = 0;
    let mut outputs: Vec<String> = Vec::new();
    while pos < graph.len() {
        let field = read_field(graph, &mut pos)?;
        if field.num == 1 && field.wire == WIRE_LEN {
            let index = node_index;
            node_index += 1;
            if let Some(c) = candidates
                .iter()
                .find(|c| c.index == index && c.skip.is_none())
            {
                let k = specs.len();
                let input = format!("{NOISE_INPUT_PREFIX}{k}");
                for node in replacement_nodes(k, &input, &c.node) {
                    put_len_field(&mut out, 1, &node);
                }
                let (size, count_output) = match &c.node.shape {
                    ShapeFrom::Fixed(dims) => {
                        let n = dims.iter().product::<i64>().max(0) as usize;
                        (NoiseSize::Fixed(n), None)
                    }
                    ShapeFrom::Like(_) => {
                        let n = count_name(k);
                        outputs.push(n.clone());
                        (NoiseSize::Like, Some(n))
                    }
                };
                specs.push(NoiseSpec {
                    input,
                    dist: c.node.dist,
                    size,
                    count_output,
                });
                continue;
            }
        }
        out.extend_from_slice(field.raw);
    }
    for spec in specs.iter() {
        put_len_field(&mut out, 11, &float_vector_input(&spec.input));
    }
    for name in &outputs {
        put_len_field(&mut out, 12, &int64_count_output(name));
    }
    Ok(out)
}

/// The graph output reporting how many values op `k` drew.
fn count_name(k: usize) -> String {
    format!("resonance_rn{k}_n")
}

/// A random node the rewrite replaces.
struct RandomNode {
    /// What fixes its shape: the `*Like` op's input, or its `shape`
    /// attribute.
    shape: ShapeFrom,
    output: String,
    dist: NoiseDist,
}

enum ShapeFrom {
    Like(String),
    Fixed(Vec<i64>),
}

/// What the scan reads of a `NodeProto`.
struct ParsedNode<'a> {
    op: String,
    inputs: Vec<String>,
    outputs: Vec<String>,
    attrs: Vec<Attr>,
    /// Graph-valued attributes (`If` branches, `Loop` / `Scan` bodies).
    subgraphs: Vec<&'a [u8]>,
}

impl ParsedNode<'_> {
    fn attr(&self, name: &str) -> Option<&Attr> {
        self.attrs.iter().find(|a| a.name == name)
    }

    /// The random node this is, if it is one the rewrite handles.
    fn random(&self) -> Result<Option<RandomNode>> {
        let (normal, like) = match self.op.as_str() {
            "RandomNormalLike" => (true, true),
            "RandomUniformLike" => (false, true),
            "RandomNormal" => (true, false),
            "RandomUniform" => (false, false),
            _ => return Ok(None),
        };
        let op = &self.op;
        let float = |name: &str, default: f32| self.attr(name).and_then(|a| a.f).unwrap_or(default);
        let Some(output) = self.outputs.first() else {
            bail!("{op} node without an output");
        };
        let shape = if like {
            let Some(input) = self.inputs.first() else {
                bail!("{op} node without an input");
            };
            ShapeFrom::Like(input.clone())
        } else {
            match self.attr("shape") {
                Some(a) if !a.ints.is_empty() => ShapeFrom::Fixed(a.ints.clone()),
                _ => bail!("{op} node without a shape"),
            }
        };
        let dist = if normal {
            NoiseDist::Normal {
                mean: float("mean", 0.0),
                scale: float("scale", 1.0),
            }
        } else {
            NoiseDist::Uniform {
                low: float("low", 0.0),
                high: float("high", 1.0),
            }
        };
        Ok(Some(RandomNode {
            shape,
            output: output.clone(),
            dist,
        }))
    }
}

fn parse_node(node: &[u8]) -> Result<ParsedNode<'_>> {
    let mut parsed = ParsedNode {
        op: String::new(),
        inputs: Vec::new(),
        outputs: Vec::new(),
        attrs: Vec::new(),
        subgraphs: Vec::new(),
    };
    let mut pos = 0;
    while pos < node.len() {
        let field = read_field(node, &mut pos)?;
        match (field.num, field.wire) {
            (1, WIRE_LEN) => parsed.inputs.push(utf8(field.payload)?),
            (2, WIRE_LEN) => parsed.outputs.push(utf8(field.payload)?),
            (4, WIRE_LEN) => parsed.op = utf8(field.payload)?,
            (5, WIRE_LEN) => {
                let (attr, graphs) = parse_attribute(field.payload)?;
                parsed.subgraphs.extend(graphs);
                parsed.attrs.push(attr);
            }
            _ => {}
        }
    }
    Ok(parsed)
}

/// What the rewrite reads of an `AttributeProto`.
struct Attr {
    name: String,
    f: Option<f32>,
    i: Option<i64>,
    ints: Vec<i64>,
}

/// An attribute and its graph values (`g` (6) and `graphs` (11)).
fn parse_attribute(attr: &[u8]) -> Result<(Attr, Vec<&[u8]>)> {
    let mut out = Attr {
        name: String::new(),
        f: None,
        i: None,
        ints: Vec::new(),
    };
    let mut graphs = Vec::new();
    let mut pos = 0;
    while pos < attr.len() {
        let field = read_field(attr, &mut pos)?;
        match (field.num, field.wire) {
            (1, WIRE_LEN) => out.name = utf8(field.payload)?,
            (2, WIRE_32) => out.f = Some(f32::from_le_bytes(field.payload.try_into()?)),
            (3, WIRE_VARINT) => out.i = Some(field.varint as i64),
            (6 | 11, WIRE_LEN) => graphs.push(field.payload),
            (8, WIRE_VARINT) => out.ints.push(field.varint as i64),
            (8, WIRE_LEN) => {
                let mut p = 0;
                while p < field.payload.len() {
                    out.ints.push(read_varint(field.payload, &mut p)? as i64);
                }
            }
            _ => {}
        }
    }
    Ok((out, graphs))
}

/// The nodes standing in for random op number `k`.
fn replacement_nodes(k: usize, noise_input: &str, random: &RandomNode) -> Vec<Vec<u8>> {
    let p = format!("resonance_rn{k}");
    let shape = format!("{p}_shape");
    let n = count_name(k);
    let zero = format!("{p}_zero");
    let flat = format!("{p}_flat");
    let mut nodes = match &random.shape {
        ShapeFrom::Like(like) => vec![
            node(&format!("{p}/Shape"), "Shape", &[like], &[&shape], &[]),
            node(
                &format!("{p}/ReduceProd"),
                "ReduceProd",
                &[&shape],
                &[&n],
                &[int_attr("keepdims", 1)],
            ),
        ],
        ShapeFrom::Fixed(dims) => vec![
            node(
                &format!("{p}/Shape"),
                "Constant",
                &[],
                &[&shape],
                &[ints_attr("value_ints", dims)],
            ),
            node(
                &format!("{p}/N"),
                "Constant",
                &[],
                &[&n],
                &[ints_attr("value_ints", &[dims.iter().product()])],
            ),
        ],
    };
    nodes.extend([
        node(
            &format!("{p}/Zero"),
            "Constant",
            &[],
            &[&zero],
            &[ints_attr("value_ints", &[0])],
        ),
        node(
            &format!("{p}/Slice"),
            "Slice",
            &[noise_input, &zero, &n],
            &[&flat],
            &[],
        ),
        node(
            &format!("{p}/Reshape"),
            "Reshape",
            &[&flat, &shape],
            &[&random.output],
            &[],
        ),
    ]);
    nodes
}

fn node(name: &str, op: &str, inputs: &[&str], outputs: &[&str], attrs: &[Vec<u8>]) -> Vec<u8> {
    let mut out = Vec::new();
    for i in inputs {
        put_len_field(&mut out, 1, i.as_bytes());
    }
    for o in outputs {
        put_len_field(&mut out, 2, o.as_bytes());
    }
    put_len_field(&mut out, 3, name.as_bytes());
    put_len_field(&mut out, 4, op.as_bytes());
    for a in attrs {
        put_len_field(&mut out, 5, a);
    }
    out
}

/// `AttributeProto` type ids.
const ATTR_INT: u64 = 2;
const ATTR_INTS: u64 = 7;

fn int_attr(name: &str, value: i64) -> Vec<u8> {
    let mut out = Vec::new();
    put_len_field(&mut out, 1, name.as_bytes());
    put_varint_field(&mut out, 3, value as u64);
    put_varint_field(&mut out, 20, ATTR_INT);
    out
}

fn ints_attr(name: &str, values: &[i64]) -> Vec<u8> {
    let mut out = Vec::new();
    put_len_field(&mut out, 1, name.as_bytes());
    for v in values {
        put_varint_field(&mut out, 8, *v as u64);
    }
    put_varint_field(&mut out, 20, ATTR_INTS);
    out
}

/// `ValueInfoProto` of a 1-D float graph input of symbolic length.
fn float_vector_input(name: &str) -> Vec<u8> {
    let mut dim = Vec::new();
    put_len_field(&mut dim, 2, b"resonance_noise_len");
    tensor_info(name, 1, &dim)
}

/// `ValueInfoProto` of a one-element int64 tensor (`[1]`).
fn int64_count_output(name: &str) -> Vec<u8> {
    let mut dim = Vec::new();
    put_varint_field(&mut dim, 1, 1);
    tensor_info(name, 7, &dim)
}

fn tensor_info(name: &str, elem_type: u64, dim: &[u8]) -> Vec<u8> {
    let mut shape = Vec::new();
    put_len_field(&mut shape, 1, dim);
    let mut tensor = Vec::new();
    put_varint_field(&mut tensor, 1, elem_type);
    put_len_field(&mut tensor, 2, &shape);
    let mut ty = Vec::new();
    put_len_field(&mut ty, 1, &tensor);
    let mut out = Vec::new();
    put_len_field(&mut out, 1, name.as_bytes());
    put_len_field(&mut out, 2, &ty);
    out
}

/// `len` samples of seeded noise for input number `stream` under `seed`.
///
/// A SplitMix64 counter stream per `(seed, stream)`; normals by
/// Box–Muller. Prefix-stable: the first `n` values do not depend on `len`.
pub fn noise(seed: u64, stream: u64, len: usize, dist: NoiseDist) -> Vec<f32> {
    let mut state = seed ^ stream.wrapping_add(1).wrapping_mul(0xA076_1D64_78BD_642F);
    let mut next_unit = move || {
        // SplitMix64, 53-bit mantissa in (0, 1].
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^= z >> 31;
        ((z >> 11) as f64 + 1.0) / (1u64 << 53) as f64
    };
    let mut out = Vec::with_capacity(len);
    match dist {
        NoiseDist::Uniform { low, high } => {
            while out.len() < len {
                let u = next_unit() - f64::EPSILON;
                out.push((low as f64 + (high - low) as f64 * u.max(0.0)) as f32);
            }
        }
        NoiseDist::Normal { mean, scale } => {
            while out.len() < len {
                let (u1, u2) = (next_unit(), next_unit());
                let r = (-2.0 * u1.ln()).sqrt();
                let theta = std::f64::consts::TAU * u2;
                out.push((mean as f64 + scale as f64 * r * theta.cos()) as f32);
                if out.len() < len {
                    out.push((mean as f64 + scale as f64 * r * theta.sin()) as f32);
                }
            }
        }
    }
    out
}

// ---- protobuf wire format ----

const WIRE_VARINT: u8 = 0;
const WIRE_64: u8 = 1;
const WIRE_LEN: u8 = 2;
const WIRE_32: u8 = 5;

struct Field<'a> {
    num: u64,
    wire: u8,
    /// The whole field, tag included.
    raw: &'a [u8],
    /// A length-delimited or fixed field's bytes.
    payload: &'a [u8],
    varint: u64,
}

fn read_varint(buf: &[u8], pos: &mut usize) -> Result<u64> {
    let mut value = 0u64;
    for shift in (0..64).step_by(7) {
        let byte = *buf
            .get(*pos)
            .ok_or_else(|| anyhow!("truncated varint in ONNX model"))?;
        *pos += 1;
        value |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    bail!("varint longer than 10 bytes in ONNX model")
}

fn read_field<'a>(buf: &'a [u8], pos: &mut usize) -> Result<Field<'a>> {
    let start = *pos;
    let tag = read_varint(buf, pos)?;
    let wire = (tag & 7) as u8;
    let num = tag >> 3;
    let mut varint = 0;
    let payload = match wire {
        WIRE_VARINT => {
            varint = read_varint(buf, pos)?;
            &buf[*pos..*pos]
        }
        WIRE_64 | WIRE_32 => {
            let n = if wire == WIRE_64 { 8 } else { 4 };
            let end = *pos + n;
            let bytes = buf
                .get(*pos..end)
                .ok_or_else(|| anyhow!("truncated fixed field in ONNX model"))?;
            *pos = end;
            bytes
        }
        WIRE_LEN => {
            let len = read_varint(buf, pos)? as usize;
            let end = pos
                .checked_add(len)
                .filter(|e| *e <= buf.len())
                .ok_or_else(|| anyhow!("truncated length-delimited field in ONNX model"))?;
            let bytes = &buf[*pos..end];
            *pos = end;
            bytes
        }
        other => bail!("unsupported protobuf wire type {other} in ONNX model"),
    };
    Ok(Field {
        num,
        wire,
        raw: &buf[start..*pos],
        payload,
        varint,
    })
}

fn put_varint(out: &mut Vec<u8>, mut v: u64) {
    while v >= 0x80 {
        out.push((v as u8) | 0x80);
        v >>= 7;
    }
    out.push(v as u8);
}

fn put_varint_field(out: &mut Vec<u8>, num: u64, v: u64) {
    put_varint(out, num << 3 | WIRE_VARINT as u64);
    put_varint(out, v);
}

fn put_len_field(out: &mut Vec<u8>, num: u64, bytes: &[u8]) {
    put_varint(out, num << 3 | WIRE_LEN as u64);
    put_varint(out, bytes.len() as u64);
    out.extend_from_slice(bytes);
}

fn utf8(bytes: &[u8]) -> Result<String> {
    Ok(std::str::from_utf8(bytes)?.to_owned())
}
