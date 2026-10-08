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
//! `s` and `n` constants), `noise_k` being a new float graph input. The caller feeds it from
//! [`noise`], a counter-based generator keyed by a seed it chooses (the
//! render unit's content key), so a unit renders bit-identically however
//! often and in whatever order it is rendered. The buffer only has to be
//! at least as long as the tensor the op would have drawn; the stream is
//! prefix-stable, so its length never changes the values.
//!
//! The rewrite works on the protobuf wire format directly (no ONNX crate):
//! `ModelProto.graph` (field 7) → `GraphProto.node` (1) / `.input` (11).
//! Everything else — initializers, opsets, subgraphs — is copied byte for
//! byte.

use anyhow::{anyhow, bail, Result};

/// The distribution one replaced op drew from.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum NoiseDist {
    Normal { mean: f32, scale: f32 },
    Uniform { low: f32, high: f32 },
}

/// One noise input [`seed_random_ops`] added: its graph input name and
/// what to fill it with.
#[derive(Debug, Clone, PartialEq)]
pub struct NoiseSpec {
    pub input: String,
    pub dist: NoiseDist,
}

/// Prefix of the graph inputs [`seed_random_ops`] adds.
pub const NOISE_INPUT_PREFIX: &str = "resonance_noise_";

/// Rewrite `model` (a serialized `ModelProto`) so its top-level
/// `RandomNormal(Like)` / `RandomUniform(Like)` ops read a fed noise input
/// instead of the runtime's clock-seeded generator. Returns the new bytes
/// and one [`NoiseSpec`] per replaced op, in graph order. A model with no
/// such op comes back unchanged with no specs. An op with a non-float
/// `dtype` is left alone (none of the shipped banks has one).
pub fn seed_random_ops(model: &[u8]) -> Result<(Vec<u8>, Vec<NoiseSpec>)> {
    let mut out = Vec::with_capacity(model.len() + 1024);
    let mut specs = Vec::new();
    let mut pos = 0;
    while pos < model.len() {
        let field = read_field(model, &mut pos)?;
        if field.num == 7 && field.wire == WIRE_LEN {
            let graph = rewrite_graph(field.payload, &mut specs)?;
            put_len_field(&mut out, 7, &graph);
        } else {
            out.extend_from_slice(field.raw);
        }
    }
    if specs.is_empty() {
        return Ok((model.to_vec(), specs));
    }
    Ok((out, specs))
}

fn rewrite_graph(graph: &[u8], specs: &mut Vec<NoiseSpec>) -> Result<Vec<u8>> {
    let first = specs.len();
    let mut out = Vec::with_capacity(graph.len() + 1024);
    let mut pos = 0;
    while pos < graph.len() {
        let field = read_field(graph, &mut pos)?;
        if field.num == 1 && field.wire == WIRE_LEN {
            if let Some(random) = parse_random_node(field.payload)? {
                let k = specs.len();
                let input = format!("{NOISE_INPUT_PREFIX}{k}");
                for node in replacement_nodes(k, &input, &random) {
                    put_len_field(&mut out, 1, &node);
                }
                specs.push(NoiseSpec {
                    input,
                    dist: random.dist,
                });
                continue;
            }
        }
        out.extend_from_slice(field.raw);
    }
    for spec in &specs[first..] {
        put_len_field(&mut out, 11, &float_vector_input(&spec.input));
    }
    Ok(out)
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

fn parse_random_node(node: &[u8]) -> Result<Option<RandomNode>> {
    let mut inputs = Vec::new();
    let mut outputs = Vec::new();
    let mut op_type = String::new();
    let mut attrs: Vec<Attr> = Vec::new();
    let mut pos = 0;
    while pos < node.len() {
        let field = read_field(node, &mut pos)?;
        match (field.num, field.wire) {
            (1, WIRE_LEN) => inputs.push(utf8(field.payload)?),
            (2, WIRE_LEN) => outputs.push(utf8(field.payload)?),
            (4, WIRE_LEN) => op_type = utf8(field.payload)?,
            (5, WIRE_LEN) => attrs.push(parse_attribute(field.payload)?),
            _ => {}
        }
    }
    let (normal, like) = match op_type.as_str() {
        "RandomNormalLike" => (true, true),
        "RandomUniformLike" => (false, true),
        "RandomNormal" => (true, false),
        "RandomUniform" => (false, false),
        _ => return Ok(None),
    };
    let attr = |name: &str| attrs.iter().find(|a| a.name == name);
    let float = |name: &str, default: f32| attr(name).and_then(|a| a.f).unwrap_or(default);
    let dtype = attr("dtype").and_then(|a| a.i);
    if dtype.is_some_and(|d| d != 1) {
        tracing::warn!("{op_type} with dtype {dtype:?} left unseeded");
        return Ok(None);
    }
    let Some(output) = outputs.first() else {
        bail!("{op_type} node without an output");
    };
    let shape = if like {
        let Some(input) = inputs.first() else {
            bail!("{op_type} node without an input");
        };
        ShapeFrom::Like(input.clone())
    } else {
        match attr("shape") {
            Some(a) if !a.ints.is_empty() => ShapeFrom::Fixed(a.ints.clone()),
            _ => bail!("{op_type} node without a shape"),
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

/// What the rewrite reads of an `AttributeProto`.
struct Attr {
    name: String,
    f: Option<f32>,
    i: Option<i64>,
    ints: Vec<i64>,
}

fn parse_attribute(attr: &[u8]) -> Result<Attr> {
    let mut out = Attr {
        name: String::new(),
        f: None,
        i: None,
        ints: Vec::new(),
    };
    let mut pos = 0;
    while pos < attr.len() {
        let field = read_field(attr, &mut pos)?;
        match (field.num, field.wire) {
            (1, WIRE_LEN) => out.name = utf8(field.payload)?,
            (2, WIRE_32) => out.f = Some(f32::from_le_bytes(field.payload.try_into()?)),
            (3, WIRE_VARINT) => out.i = Some(field.varint as i64),
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
    Ok(out)
}

/// The nodes standing in for random op number `k`.
fn replacement_nodes(k: usize, noise_input: &str, random: &RandomNode) -> Vec<Vec<u8>> {
    let p = format!("resonance_rn{k}");
    let shape = format!("{p}_shape");
    let n = format!("{p}_n");
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
    let mut shape = Vec::new();
    put_len_field(&mut shape, 1, &dim);
    let mut tensor = Vec::new();
    put_varint_field(&mut tensor, 1, 1); // FLOAT
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
