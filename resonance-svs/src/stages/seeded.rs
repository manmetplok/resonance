//! A session whose in-graph random ops read fed, seeded noise
//! ([`super::noise`]), and how much noise each run is fed.
//!
//! **Loading.** The model file is read, rewritten in memory and built from
//! the rewritten bytes; the original bytes are dropped before the session
//! is built and the rewritten ones right after, so a load peaks at about
//! twice the model's size on our side, not three times. A model with
//! nothing to rewrite is built straight from its file (no copy kept). When
//! the rewrite cannot parse the model, or the rewritten model does not
//! build, the file is loaded as it is — it renders, but not repeatably
//! ([`SeededSession::repeatable`]), and a warning says so once per model.
//!
//! **Sizing.** A fixed-shape op draws a known number of values and gets
//! exactly that many. A `*Like` op draws as many as its input has, which
//! only the run knows; the graph reports it ([`super::noise::NoiseSpec::count_output`]),
//! and the session learns the count as a function of the run's frame
//! count (`a · frames + b`, from the first runs). Until it knows, a run is
//! fed `frames × bound_per_frame + 1024` values; once it does, exactly the
//! count. A guess that turns out short fails the run's `Reshape`, and the
//! run is retried once with the bound. The stream is prefix-stable, so the
//! values the model reads never depend on how many it was fed.

use std::path::Path;

use anyhow::{anyhow, Result};
use ort::session::{Session, SessionOutputs};
use ort::value::Value;

use super::common::{build_session_from, warn_once, ExecutionProvider, ModelSource};
use super::noise::{noise, seed_random_ops, NoiseSize, NoiseSpec};

/// A session whose in-graph random ops were replaced by fed noise
/// ([`super::noise::seed_random_ops`]), so a run is a pure function of
/// its inputs and the seed.
pub struct SeededSession {
    pub session: Session,
    pub noise: Vec<NoiseSpec>,
    /// The backend it runs on.
    pub backend: ExecutionProvider,
    /// Whether every random op reads fed noise: `false` when some stayed
    /// with the runtime's generator (see [`super::noise`]) or the rewrite
    /// failed and the model was loaded as it is.
    pub repeatable: bool,
    /// Per noise input: what the runs so far say about how many values it
    /// draws.
    draws: Vec<Draws>,
}

/// What a `*Like` op's draw count is known to be, as a function of the
/// run's frame count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Draws {
    Unknown,
    /// One observation.
    Seen {
        frames: usize,
        n: usize,
    },
    /// `a · frames + b`, from two observations at different frame counts.
    Affine {
        a: i64,
        b: i64,
    },
}

impl Draws {
    /// The count for a run of `frames`, when it is known or can be
    /// guessed (one observation that is a whole multiple of its frames).
    fn predict(self, frames: usize) -> Option<usize> {
        match self {
            Draws::Unknown => None,
            Draws::Seen { frames: f, n } if f == frames => Some(n),
            Draws::Seen { frames: f, n } if f > 0 && n % f == 0 => Some(n / f * frames),
            Draws::Seen { .. } => None,
            Draws::Affine { a, b } => usize::try_from(a * frames as i64 + b).ok(),
        }
    }

    fn observe(self, frames: usize, n: usize) -> Draws {
        let seen = Draws::Seen { frames, n };
        match self {
            Draws::Seen { frames: f, n: m } if f != frames => {
                let (df, dn) = (frames as i64 - f as i64, n as i64 - m as i64);
                if dn % df == 0 && dn / df >= 0 {
                    let a = dn / df;
                    Draws::Affine {
                        a,
                        b: m as i64 - a * f as i64,
                    }
                } else {
                    seen
                }
            }
            Draws::Affine { .. } if self.predict(frames) == Some(n) => self,
            _ => seen,
        }
    }
}

impl SeededSession {
    /// Read `model_path`, seed its random ops and build the session. See
    /// the module docs for the fallback when the rewrite or the rewritten
    /// model fails.
    pub fn load(model_path: &Path, ep: ExecutionProvider, device_index: i32) -> Result<Self> {
        let bytes = std::fs::read(model_path)
            .map_err(|e| anyhow!("reading ONNX model at {}: {e}", model_path.display()))?;
        let seeded = seed_random_ops(&bytes);
        drop(bytes);
        let seeded = match seeded {
            Ok(seeded) => seeded,
            Err(e) => {
                warn_once(
                    model_path,
                    format!("its random ops could not be seeded ({e:#}); loaded unseeded, renders are not repeatable"),
                );
                return Self::unseeded(model_path, ep, device_index);
            }
        };
        if !seeded.unseeded.is_empty() {
            warn_once(
                model_path,
                format!(
                    "{} random op(s) left unseeded ({}); renders are not repeatable",
                    seeded.unseeded.len(),
                    seeded.unseeded.join(", ")
                ),
            );
        }
        let repeatable = seeded.repeatable();
        let Some(rewritten) = seeded.bytes else {
            let (session, backend) =
                build_session_from(ModelSource::File(model_path), ep, device_index)?;
            return Ok(Self::new(session, Vec::new(), backend, repeatable));
        };
        let built = build_session_from(
            ModelSource::Bytes {
                path: model_path,
                bytes: &rewritten,
            },
            ep,
            device_index,
        );
        drop(rewritten);
        match built {
            Ok((session, backend)) => Ok(Self::new(session, seeded.specs, backend, repeatable)),
            Err(e) => {
                warn_once(
                    model_path,
                    format!("the seeded model does not build ({e:#}); loaded unseeded, renders are not repeatable"),
                );
                Self::unseeded(model_path, ep, device_index)
            }
        }
    }

    fn unseeded(model_path: &Path, ep: ExecutionProvider, device_index: i32) -> Result<Self> {
        let (session, backend) =
            build_session_from(ModelSource::File(model_path), ep, device_index)?;
        Ok(Self::new(session, Vec::new(), backend, false))
    }

    fn new(
        session: Session,
        noise: Vec<NoiseSpec>,
        backend: ExecutionProvider,
        repeatable: bool,
    ) -> Self {
        let draws = vec![Draws::Unknown; noise.len()];
        Self {
            session,
            noise,
            backend,
            repeatable,
            draws,
        }
    }

    /// How many noise values each input gets on a run of `frames`: the
    /// exact count where it is known, else `frames × bound_per_frame +
    /// 1024`.
    pub fn noise_lengths(&self, frames: usize, bound_per_frame: usize) -> Vec<usize> {
        self.lengths(frames, bound_per_frame, true)
    }

    fn lengths(&self, frames: usize, bound_per_frame: usize, learned: bool) -> Vec<usize> {
        let bound = frames * bound_per_frame + 1024;
        self.noise
            .iter()
            .zip(&self.draws)
            .map(|(spec, draws)| match spec.size {
                NoiseSize::Fixed(n) => n,
                NoiseSize::Like if learned => draws.predict(frames).unwrap_or(bound),
                NoiseSize::Like => bound,
            })
            .collect()
    }

    /// Run the session on `inputs` plus this seed's noise for a run of
    /// `frames` (see the module docs for the sizing), and read what is
    /// needed of the outputs with `extract`.
    pub fn run<T>(
        &mut self,
        inputs: &[(String, Value)],
        seed: u64,
        frames: usize,
        bound_per_frame: usize,
        extract: impl FnOnce(&SessionOutputs<'_>) -> Result<T>,
    ) -> Result<T> {
        let mut extract = Some(extract);
        let guessed = self.lengths(frames, bound_per_frame, true);
        let bounded = self.lengths(frames, bound_per_frame, false);
        let attempts = if guessed == bounded {
            vec![bounded]
        } else {
            vec![guessed, bounded]
        };
        let last = attempts.len() - 1;
        for (attempt, lengths) in attempts.into_iter().enumerate() {
            let noise_inputs = noise_inputs(&self.noise, seed, &lengths)?;
            let views: Vec<(&str, &Value)> = inputs
                .iter()
                .chain(noise_inputs.iter())
                .map(|(name, value)| (name.as_str(), value))
                .collect();
            match self.session.run(views) {
                Ok(outputs) => {
                    let counts: Vec<Option<usize>> = self
                        .noise
                        .iter()
                        .map(|spec| {
                            let value = outputs.get(spec.count_output.as_deref()?)?;
                            let (_, n) = value.try_extract_tensor::<i64>().ok()?;
                            n.first().and_then(|n| usize::try_from(*n).ok())
                        })
                        .collect();
                    let value = (extract.take().expect("extracted once"))(&outputs);
                    drop(outputs);
                    for (draws, count) in self.draws.iter_mut().zip(counts) {
                        if let Some(n) = count {
                            *draws = draws.observe(frames, n);
                        }
                    }
                    return value;
                }
                Err(e) if attempt < last => {
                    tracing::debug!(
                        "seeded run with learned noise sizes failed ({e}); retrying with the bound"
                    );
                    self.draws.iter_mut().for_each(|d| *d = Draws::Unknown);
                }
                Err(e) => return Err(e.into()),
            }
        }
        unreachable!("the bounded attempt returns")
    }

    /// The noise inputs for one run under `seed`: each at least
    /// `per_frame × n_frames` long (+1024), a fixed-shape one exactly its
    /// size. The stream is prefix-stable, so the over-allocation does not
    /// change the values; a model that draws more than that fails its
    /// `Reshape` loudly.
    pub fn noise_values(
        specs: &[NoiseSpec],
        seed: u64,
        n_frames: usize,
        per_frame: usize,
    ) -> Result<Vec<(String, Value)>> {
        let lengths: Vec<usize> = specs
            .iter()
            .map(|spec| match spec.size {
                NoiseSize::Fixed(n) => n,
                NoiseSize::Like => n_frames * per_frame + 1024,
            })
            .collect();
        noise_inputs(specs, seed, &lengths)
    }
}

/// Input `k` of `specs` filled with `lengths[k]` values of stream `k`
/// under `seed`.
pub fn noise_inputs(
    specs: &[NoiseSpec],
    seed: u64,
    lengths: &[usize],
) -> Result<Vec<(String, Value)>> {
    specs
        .iter()
        .zip(lengths)
        .enumerate()
        .map(|(k, (spec, &len))| {
            let data = noise(seed, k as u64, len, spec.dist);
            let arr = ndarray::Array1::<f32>::from_vec(data);
            Ok((spec.input.clone(), Value::from_array(arr)?.into()))
        })
        .collect()
}
