//! Execution provider settings shared by the ONNX engines.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// The GPU execution provider an ONNX engine runs on.
#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum OnnxGpuProvider {
    /// ONNX Runtime's CUDA provider (or MIGraphX on AMD builds).
    #[default]
    Cuda,
    /// NVIDIA TensorRT, with CUDA under it for the nodes TensorRT does not
    /// take. Needs a binary built with a `*-tensorrt` feature and the
    /// TensorRT 10 libraries (`libnvinfer.so.10`) on the library path.
    Tensorrt,
}

impl std::str::FromStr for OnnxGpuProvider {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cuda" => Ok(Self::Cuda),
            "tensorrt" | "trt" => Ok(Self::Tensorrt),
            other => Err(format!(
                "unknown ONNX GPU provider '{other}' (use cuda or tensorrt)"
            )),
        }
    }
}

/// `[onnx]`: how the ONNX engines (Cohere, Moonshine, SenseVoice and the
/// rest) use the GPU. The defaults keep the CUDA provider.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct OnnxConfig {
    /// `cuda` (default) or `tensorrt`.
    #[serde(default)]
    pub gpu_provider: OnnxGpuProvider,

    /// TensorRT only: let TensorRT pick fp16 kernels. Off: fp32 throughout.
    #[serde(default = "default_true")]
    pub tensorrt_fp16: bool,

    /// TensorRT only: where built engines and the timing cache are kept.
    /// Unset: `$XDG_CACHE_HOME/voxtype/tensorrt` (`~/.cache/voxtype/tensorrt`).
    #[serde(default)]
    pub tensorrt_cache_dir: Option<PathBuf>,
}

fn default_true() -> bool {
    true
}

impl Default for OnnxConfig {
    fn default() -> Self {
        Self {
            gpu_provider: OnnxGpuProvider::Cuda,
            tensorrt_fp16: true,
            tensorrt_cache_dir: None,
        }
    }
}

impl OnnxConfig {
    /// The TensorRT cache folder with the default filled in.
    pub fn tensorrt_cache_root(&self) -> PathBuf {
        match &self.tensorrt_cache_dir {
            Some(dir) => expand_home(dir),
            None => dirs::cache_dir()
                .unwrap_or_else(|| PathBuf::from(".cache"))
                .join("voxtype")
                .join("tensorrt"),
        }
    }
}

fn expand_home(path: &std::path::Path) -> PathBuf {
    match path.strip_prefix("~") {
        Ok(rest) => dirs::home_dir().map_or_else(|| path.to_path_buf(), |h| h.join(rest)),
        Err(_) => path.to_path_buf(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_keep_cuda() {
        let c: OnnxConfig = toml::from_str("").unwrap();
        assert_eq!(c, OnnxConfig::default());
        assert_eq!(c.gpu_provider, OnnxGpuProvider::Cuda);
        assert!(c.tensorrt_fp16);
    }

    #[test]
    fn parses_tensorrt() {
        let c: OnnxConfig =
            toml::from_str("gpu_provider = \"tensorrt\"\ntensorrt_fp16 = false").unwrap();
        assert_eq!(c.gpu_provider, OnnxGpuProvider::Tensorrt);
        assert!(!c.tensorrt_fp16);
        assert_eq!("TensorRT".parse(), Ok(OnnxGpuProvider::Tensorrt));
        assert!("rocm".parse::<OnnxGpuProvider>().is_err());
    }

    #[test]
    fn cache_dir_expands_home() {
        let c = OnnxConfig {
            tensorrt_cache_dir: Some(PathBuf::from("~/trt")),
            ..OnnxConfig::default()
        };
        assert!(!c.tensorrt_cache_root().starts_with("~"));
        assert!(OnnxConfig::default()
            .tensorrt_cache_root()
            .ends_with("voxtype/tensorrt"));
    }
}
