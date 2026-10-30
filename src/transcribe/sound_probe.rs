//! The sound strip's phone probe (ASR-10): a linear layer on one inner
//! layer of the live model's encoder that names the speech sound (one of 41
//! phone classes, silence included) in each 80 ms frame.
//!
//! `scripts/sound-probe-export.py` writes it as `sound-probe.bin` next to
//! the live model, together with `encoder-sounds.onnx`, the encoder with
//! that layer as a second output. The file, all little endian:
//!
//! ```text
//! b"VXSP", u32 version (1), u32 dim, u32 classes,
//! the encoder output name and then each class label, each as u32 length + UTF-8,
//! f32 weight[classes][dim], f32 bias[classes]
//! ```
//!
//! The feature standardisation the probe was trained with is folded into
//! the weights, so a frame goes in as the encoder gives it.

use std::path::Path;

const MAGIC: &[u8; 4] = b"VXSP";
const VERSION: u32 = 1;

/// The file name next to the live model.
pub const PROBE_FILE: &str = "sound-probe.bin";
/// The encoder with the probed layer as an extra output, next to the live
/// model.
pub const ENCODER_FILE: &str = "encoder-sounds.onnx";
/// The label of the silence class.
pub const SILENCE: &str = "sil";

pub struct SoundProbe {
    /// The encoder output the probe reads.
    pub output: String,
    pub labels: Vec<String>,
    dim: usize,
    weight: Vec<f32>,
    bias: Vec<f32>,
}

impl SoundProbe {
    pub fn load(path: &Path) -> Result<Self, String> {
        let bytes = std::fs::read(path).map_err(|e| format!("{}: {}", path.display(), e))?;
        Self::parse(&bytes).map_err(|e| format!("{}: {}", path.display(), e))
    }

    fn parse(bytes: &[u8]) -> Result<Self, String> {
        let mut r = Reader { bytes, at: 0 };
        if r.take(4)? != MAGIC {
            return Err("not a sound probe file".into());
        }
        let version = r.u32()?;
        if version != VERSION {
            return Err(format!(
                "version {} (this voxtype reads {})",
                version, VERSION
            ));
        }
        let dim = r.u32()? as usize;
        let classes = r.u32()? as usize;
        let mut strings = Vec::with_capacity(classes + 1);
        for _ in 0..=classes {
            let n = r.u32()? as usize;
            let s = std::str::from_utf8(r.take(n)?).map_err(|_| "a label is not UTF-8")?;
            strings.push(s.to_string());
        }
        let weight = r.floats(classes * dim)?;
        let bias = r.floats(classes)?;
        if r.at != bytes.len() {
            return Err("bytes after the bias".into());
        }
        let output = strings.remove(0);
        Ok(Self {
            output,
            labels: strings,
            dim,
            weight,
            bias,
        })
    }

    /// The features one frame needs.
    pub fn dim(&self) -> usize {
        self.dim
    }

    /// The `k` likeliest classes of one frame (`dim` features), likeliest
    /// first, as (class, natural log probability).
    pub fn top(&self, frame: &[f32], k: usize) -> Vec<(usize, f32)> {
        debug_assert_eq!(frame.len(), self.dim);
        let logits: Vec<f32> = self
            .weight
            .chunks_exact(self.dim)
            .zip(&self.bias)
            .map(|(w, b)| b + w.iter().zip(frame).map(|(a, x)| a * x).sum::<f32>())
            .collect();
        let max = logits.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let log_sum = max + logits.iter().map(|l| (l - max).exp()).sum::<f32>().ln();
        let mut order: Vec<usize> = (0..logits.len()).collect();
        order.sort_by(|&a, &b| logits[b].total_cmp(&logits[a]));
        order
            .into_iter()
            .take(k)
            .map(|c| (c, logits[c] - log_sum))
            .collect()
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], String> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.bytes.len());
        let end = end.ok_or("file ends early")?;
        let s = &self.bytes[self.at..end];
        self.at = end;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32, String> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn floats(&mut self, n: usize) -> Result<Vec<f32>, String> {
        let len = n.checked_mul(4).ok_or("size overflows")?;
        Ok(self
            .take(len)?
            .as_chunks::<4>()
            .0
            .iter()
            .map(|b| f32::from_le_bytes(*b))
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(dim: u32, labels: &[&str], weight: &[f32], bias: &[f32]) -> Vec<u8> {
        let mut b = MAGIC.to_vec();
        for v in [VERSION, dim, labels.len() as u32] {
            b.extend(v.to_le_bytes());
        }
        for s in std::iter::once(&"/layers.8/out").chain(labels) {
            b.extend((s.len() as u32).to_le_bytes());
            b.extend(s.as_bytes());
        }
        for f in weight.iter().chain(bias) {
            b.extend(f.to_le_bytes());
        }
        b
    }

    #[test]
    fn a_probe_names_the_likeliest_sounds_with_their_log_probability() {
        // Two features, three classes: "sil" follows the first, "AA" the
        // second, "S" neither.
        let bytes = file(
            2,
            &["sil", "AA", "S"],
            &[2.0, 0.0, 0.0, 2.0, 0.0, 0.0],
            &[0.0, 0.0, 0.5],
        );
        let p = SoundProbe::parse(&bytes).unwrap();
        assert_eq!(p.output, "/layers.8/out");
        assert_eq!(p.labels, ["sil", "AA", "S"]);
        assert_eq!(p.dim(), 2);
        let top = p.top(&[0.0, 3.0], 2);
        assert_eq!(top.iter().map(|t| t.0).collect::<Vec<_>>(), [1, 2]);
        // softmax of [0, 6, 0.5]
        let z = 1.0f32 + 6.0f32.exp() + 0.5f32.exp();
        assert!((top[0].1 - (6.0 - z.ln())).abs() < 1e-5);
        assert!((top[1].1 - (0.5 - z.ln())).abs() < 1e-5);
    }

    #[test]
    fn a_short_or_foreign_file_is_refused() {
        let bytes = file(2, &["sil"], &[1.0, 1.0], &[0.0]);
        assert!(SoundProbe::parse(&bytes[..bytes.len() - 1]).is_err());
        let mut long = bytes.clone();
        long.push(0);
        assert!(SoundProbe::parse(&long).is_err());
        assert!(SoundProbe::parse(b"GGUF").is_err());
    }
}
