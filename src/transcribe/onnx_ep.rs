//! Shared helper that registers ONNX Runtime execution providers for any
//! ONNX-backed engine in voxtype.
//!
//! Each engine's session builder calls [`register_gpu_eps`] to attach the
//! GPU EPs that were compiled into this binary. The compile-time gating
//! lives on three marker features in `Cargo.toml`:
//!
//! - `onnx-cuda-enabled`     — CUDA EP (NVIDIA)
//! - `onnx-migraphx-enabled` — MIGraphX EP (AMD)
//! - `onnx-tensorrt-enabled` — TensorRT EP (NVIDIA, optimized)
//!
//! Enabling any per-engine feature like `cohere-migraphx` or
//! `parakeet-cuda` transitively enables the matching marker, so this
//! helper sees the right EPs without each engine duplicating the cfg
//! plumbing.
//!
//! TensorRT is registered only when `[onnx] gpu_provider = "tensorrt"`
//! (see [`set_options`]) and `libnvinfer.so.10` loads. It then builds
//! engines with fp16 kernels, keeps them and a timing cache on disk, and
//! takes an explicit input profile where the engine gives one, so a worker
//! start reads its engines instead of rebuilding them.
//!
//! Order matters: ort tries EPs in sequence and falls through to the
//! next on registration failure. Specialized EPs (TensorRT) come before
//! their generic siblings (CUDA). The CPU EP is always implicit at the
//! bottom of the chain — even if every GPU EP fails to register at
//! runtime (no GPU, missing driver, missing companion .so files), ort
//! still runs the model on CPU.

#[cfg(feature = "onnx-common")]
use ort::execution_providers::ExecutionProviderDispatch;
#[cfg(feature = "onnx-common")]
use ort::session::builder::{BuilderResult, SessionBuilder};

/// Lets the session's threads sleep while they wait for work. ORT's
/// default lets them spin, which keeps whole cores busy for the same text
/// and speed.
// BuilderResult is ort's own type: its error hands the builder back.
#[cfg(feature = "onnx-common")]
#[allow(clippy::result_large_err)]
pub fn sleeping_threads(builder: SessionBuilder) -> BuilderResult {
    builder
        .with_intra_op_spinning(false)?
        .with_inter_op_spinning(false)
}

use std::sync::OnceLock;

use crate::config::{OnnxConfig, OnnxGpuProvider};

static OPTIONS: OnceLock<OnnxConfig> = OnceLock::new();

/// Set the `[onnx]` options for this process, once, after the config and
/// its overrides are final. Later calls are ignored.
pub fn set_options(options: &OnnxConfig) {
    let _ = OPTIONS.set(options.clone());
}

/// The `[onnx]` options in force: those given to [`set_options`], or the
/// defaults.
pub fn options() -> OnnxConfig {
    OPTIONS.get().cloned().unwrap_or_default()
}

/// The environment variables that carry the `[onnx]` options to a worker
/// process, so a CLI flag given to the daemon reaches the worker too.
pub fn worker_env() -> Vec<(&'static str, String)> {
    let o = options();
    let provider = match o.gpu_provider {
        OnnxGpuProvider::Cuda => "cuda",
        OnnxGpuProvider::Tensorrt => "tensorrt",
    };
    let mut env = vec![
        ("VOXTYPE_ONNX_GPU_PROVIDER", provider.to_string()),
        ("VOXTYPE_ONNX_TENSORRT_FP16", o.tensorrt_fp16.to_string()),
    ];
    if let Some(dir) = &o.tensorrt_cache_dir {
        env.push((
            "VOXTYPE_ONNX_TENSORRT_CACHE_DIR",
            dir.to_string_lossy().into_owned(),
        ));
    }
    env
}

/// The input shapes a TensorRT engine is built for, in ORT's syntax:
/// `name:d1xd2x...`, several inputs separated by commas. An input outside
/// `min..max` would make TensorRT build a new engine, so the caller must
/// keep its inputs inside the range.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrtProfile {
    pub min: String,
    pub opt: String,
    pub max: String,
}

/// How one session may use TensorRT.
#[derive(Debug, Clone, Copy)]
pub struct TrtSession<'a> {
    /// Names this session's engine folder, for example
    /// `cohere-transcribe-fp16-5f3e1a-encoder`. ORT names the engine files
    /// after the graph's structure, not its weights, so the key must change
    /// when the weights do.
    pub cache_key: &'a str,
    /// The explicit profile, or `None` to let ORT track the shapes it sees.
    pub profile: Option<&'a TrtProfile>,
}

/// Is TensorRT asked for, compiled in and loadable in this process? Logs
/// the reason once when it is asked for but cannot be used.
pub fn tensorrt_available() -> bool {
    trt::runtime().is_some()
}

#[cfg(feature = "onnx-tensorrt-enabled")]
mod trt {
    use std::ffi::CStr;
    use std::path::{Path, PathBuf};
    use std::sync::OnceLock;

    use super::{options, OnnxGpuProvider};

    /// Builder workspace cap. TensorRT's default is the whole card, which
    /// fails or pushes the desktop out on an 8 GB card.
    pub(super) const WORKSPACE_BYTES: usize = 2 << 30;

    pub(super) struct Runtime {
        /// `<root>/<card>-cc<cap>/trt<version>-<ort build>`.
        pub dir: PathBuf,
        pub fp16: bool,
    }

    pub(super) fn runtime() -> Option<&'static Runtime> {
        static RT: OnceLock<Option<Runtime>> = OnceLock::new();
        RT.get_or_init(|| {
            let o = options();
            if o.gpu_provider != OnnxGpuProvider::Tensorrt {
                return None;
            }
            match probe(&o.tensorrt_cache_root(), o.tensorrt_fp16) {
                Ok(rt) => {
                    tracing::info!(
                        "TensorRT ready: fp16={}, engine cache {}",
                        rt.fp16,
                        rt.dir.display()
                    );
                    Some(rt)
                }
                Err(e) => {
                    tracing::warn!("TensorRT could not start ({e}), so the ONNX engines use CUDA");
                    None
                }
            }
        })
        .as_ref()
    }

    fn probe(root: &Path, fp16: bool) -> Result<Runtime, String> {
        let version = nvinfer_version()?;
        let gpu = gpu_tag()?;
        let ort = ort_tag();
        let dir = root.join(gpu).join(format!("trt{version}-{ort}"));
        std::fs::create_dir_all(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        Ok(Runtime { dir, fp16 })
    }

    /// Loads `libnvinfer.so.10` the way ORT's TensorRT provider will and
    /// asks it for its version (`getInferLibVersion`, major*10000 +
    /// minor*100 + patch). The library stays loaded, which ORT needs anyway.
    fn nvinfer_version() -> Result<String, String> {
        const LIB: &CStr = c"libnvinfer.so.10";
        // SAFETY: dlopen and dlsym with valid C strings; the symbol is a
        // plain `int32_t getInferLibVersion(void)` in every TensorRT 10.
        unsafe {
            let handle = libc::dlopen(LIB.as_ptr(), libc::RTLD_NOW | libc::RTLD_GLOBAL);
            if handle.is_null() {
                let err = libc::dlerror();
                let why = if err.is_null() {
                    String::new()
                } else {
                    CStr::from_ptr(err).to_string_lossy().into_owned()
                };
                return Err(format!(
                    "libnvinfer.so.10 not found on the library path: {why}"
                ));
            }
            let sym = libc::dlsym(handle, c"getInferLibVersion".as_ptr());
            if sym.is_null() {
                return Err("libnvinfer.so.10 has no getInferLibVersion".into());
            }
            let f: extern "C" fn() -> i32 = std::mem::transmute(sym);
            let v = f();
            Ok(format!("{}.{}.{}", v / 10000, (v / 100) % 100, v % 100))
        }
    }

    /// The first card as `<name>-cc<compute capability>`, from nvidia-smi.
    /// An engine is only valid on the card model it was built for.
    fn gpu_tag() -> Result<String, String> {
        let out = std::process::Command::new("nvidia-smi")
            .args([
                "--id=0",
                "--query-gpu=name,compute_cap",
                "--format=csv,noheader",
            ])
            .output()
            .map_err(|e| format!("nvidia-smi: {e}"))?;
        let line = String::from_utf8_lossy(&out.stdout);
        match line.trim().split_once(',') {
            Some((name, cap)) if out.status.success() => {
                Ok(format!("{}-cc{}", tag(name.trim()), tag(cap.trim())))
            }
            _ => Err(format!("nvidia-smi said {:?}", line.trim())),
        }
    }

    /// The ONNX Runtime build: branch and commit from its build info, since
    /// two builds of one version (voxtype-bin's and Microsoft's) differ.
    fn ort_tag() -> String {
        let info = ort::info();
        // "ORT Build Info: git-branch=rel-1.24.4, git-commit-id=2d924974ef, ..."
        let field = |key: &str| {
            info.split([',', ' '])
                .find_map(|f| f.trim().strip_prefix(key))
                .map(|v| v.trim().to_string())
        };
        let branch = field("git-branch=").unwrap_or_else(|| "unknown".into());
        let commit = field("git-commit-id=").unwrap_or_default();
        tag(&format!("ort-{branch}-{commit}"))
    }

    pub(super) fn tag(s: &str) -> String {
        s.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '.' {
                    c
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .trim_matches('-')
            .to_string()
    }
}

#[cfg(not(feature = "onnx-tensorrt-enabled"))]
mod trt {
    use super::{options, OnnxGpuProvider};

    pub(super) enum Runtime {}

    pub(super) fn runtime() -> Option<&'static Runtime> {
        static WARNED: std::sync::Once = std::sync::Once::new();
        if options().gpu_provider == OnnxGpuProvider::Tensorrt {
            WARNED.call_once(|| {
                tracing::warn!(
                    "gpu_provider = \"tensorrt\", but this voxtype was built without \
                     TensorRT; the ONNX engines use CUDA"
                )
            });
        }
        None
    }
}

/// Register GPU EPs onto a session builder.
///
/// `engine_label` and `session_label` are used for logging
/// (`"Cohere encoder: registering execution providers [...]"`) and, with
/// TensorRT, to name the session's engine folder. Returns the modified
/// builder; if no EPs are compiled in or registration fails, falls through
/// unchanged and ort uses the CPU EP.
#[cfg(feature = "onnx-common")]
#[allow(clippy::result_large_err)]
pub fn register_gpu_eps(
    builder: SessionBuilder,
    engine_label: &str,
    session_label: &str,
) -> BuilderResult {
    let key = format!("{engine_label}-{session_label}").to_lowercase();
    let trt = TrtSession {
        cache_key: &key,
        profile: None,
    };
    register_gpu_eps_with(builder, engine_label, session_label, Some(&trt))
}

/// [`register_gpu_eps`] with the TensorRT settings spelled out. `trt =
/// None` keeps the session off TensorRT, as a fallback after TensorRT
/// failed to build or run it.
#[cfg(feature = "onnx-common")]
#[allow(clippy::result_large_err)]
pub fn register_gpu_eps_with(
    builder: SessionBuilder,
    engine_label: &str,
    session_label: &str,
    trt: Option<&TrtSession>,
) -> BuilderResult {
    let providers = compiled_providers(engine_label, session_label, trt);
    if providers.is_empty() {
        return Ok(builder);
    }
    let names: Vec<&'static str> = providers.iter().map(|(n, _)| *n).collect();
    tracing::info!("{engine_label} {session_label}: registering execution providers {names:?}");
    let dispatches: Vec<_> = providers.into_iter().map(|(_, ep)| ep).collect();
    builder.with_execution_providers(dispatches)
}

/// The file that records a TensorRT failure in a session's engine folder.
const FAILED_FILE: &str = "FAILED";

/// Why TensorRT failed for this session before, if it did. A failed build
/// costs a minute or more and floods the log with parser errors, so a
/// session that failed once stays on CUDA until the file is removed.
pub fn tensorrt_failed_before(cache_key: &str) -> Option<String> {
    let file = tensorrt_engine_dir(cache_key)?.join(FAILED_FILE);
    std::fs::read_to_string(&file).ok().map(|why| {
        format!(
            "{} (remove {} to try TensorRT again)",
            why.lines().next().unwrap_or_default(),
            file.display()
        )
    })
}

/// Records that TensorRT failed for this session; see
/// [`tensorrt_failed_before`].
pub fn mark_tensorrt_failed(cache_key: &str, why: &str) {
    let Some(dir) = tensorrt_engine_dir(cache_key) else {
        return;
    };
    let why: String = why.chars().take(4000).collect();
    if let Err(e) = std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join(FAILED_FILE), format!("{why}\n")))
    {
        tracing::warn!(
            "Cannot record the TensorRT failure in {}: {e}",
            dir.display()
        );
    }
}

/// The TensorRT engine folder a session would use, when TensorRT is in use.
pub fn tensorrt_engine_dir(cache_key: &str) -> Option<std::path::PathBuf> {
    #[cfg(feature = "onnx-tensorrt-enabled")]
    {
        trt::runtime().map(|rt| rt.dir.join(trt::tag(cache_key)))
    }
    #[cfg(not(feature = "onnx-tensorrt-enabled"))]
    {
        let _ = cache_key;
        None
    }
}

#[cfg(feature = "onnx-common")]
#[allow(unused_variables)]
fn compiled_providers(
    engine_label: &str,
    session_label: &str,
    trt: Option<&TrtSession>,
) -> Vec<(&'static str, ExecutionProviderDispatch)> {
    #[allow(unused_mut)]
    let mut providers: Vec<(&'static str, ExecutionProviderDispatch)> = Vec::new();

    #[cfg(feature = "onnx-tensorrt-enabled")]
    if let (Some(session), Some(rt)) = (trt, trt::runtime()) {
        use ort::execution_providers::TensorRTExecutionProvider;
        let dir = rt.dir.join(trt::tag(session.cache_key));
        match std::fs::create_dir_all(&dir) {
            Ok(()) => {
                let dir_s = dir.to_string_lossy().into_owned();
                let mut ep = TensorRTExecutionProvider::default()
                    .with_fp16(rt.fp16)
                    .with_engine_cache(true)
                    .with_engine_cache_path(&dir_s)
                    .with_timing_cache(true)
                    .with_timing_cache_path(rt.dir.to_string_lossy())
                    .with_max_workspace_size(trt::WORKSPACE_BYTES)
                    .with_context_memory_sharing(true);
                if let Some(p) = session.profile {
                    ep = ep
                        .with_profile_min_shapes(&p.min)
                        .with_profile_opt_shapes(&p.opt)
                        .with_profile_max_shapes(&p.max);
                }
                tracing::info!(
                    "{engine_label} {session_label}: TensorRT fp16={}, engines in {}, profile {:?}",
                    rt.fp16,
                    dir.display(),
                    session.profile.map(|p| &p.max),
                );
                providers.push(("TensorRT", ep.build()));
            }
            Err(e) => tracing::warn!(
                "{engine_label} {session_label}: cannot make {} ({e}); using CUDA",
                dir.display()
            ),
        }
    }
    #[cfg(feature = "onnx-cuda-enabled")]
    {
        use ort::ep::{cuda::ConvAlgorithmSearch, ArenaExtendStrategy, CUDA};
        // HEURISTIC: cuDNN's default EXHAUSTIVE search benchmarks every Conv
        // algorithm on each new input shape, with a large scratch buffer. That
        // runs mid-dictation the first time a part has a new length.
        // kSameAsRequested: the arena grows by what a run needs instead of
        // doubling, so it does not grab hundreds of MiB more in one step.
        providers.push((
            "CUDA",
            CUDA::default()
                .with_conv_algorithm_search(ConvAlgorithmSearch::Heuristic)
                .with_arena_extend_strategy(ArenaExtendStrategy::SameAsRequested)
                .build(),
        ));
    }
    #[cfg(feature = "onnx-migraphx-enabled")]
    {
        use ort::execution_providers::MIGraphXExecutionProvider;
        providers.push(("MIGraphX", MIGraphXExecutionProvider::default().build()));
    }

    providers
}
