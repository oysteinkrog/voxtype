//! Capture from one PipeWire node by name: `audio.device = "pipewire:<node>"`.
//!
//! cpal only opens the ALSA device names it enumerates, and the PipeWire ALSA
//! device among them follows the default source. That default can be a filter
//! chain (noise suppression whose VAD gate writes exact zeros over the start
//! of speech) or a remote-desktop microphone while a client is connected.
//! Opening the PipeWire ALSA plugin with its `NODE` argument targets one node
//! for this capture stream only. The system default, other apps and voxtype's
//! own feedback playback stay as they are.
//!
//! If the node does not exist, WirePlumber links the stream to the default
//! source instead.

use super::cpal_capture::{deliver, run_commands, CaptureCommand};
use super::resampler::StreamResampler;
use crate::error::AudioError;
use alsa::pcm::{Access, Format, HwParams, PCM};
use alsa::{Direction, ValueOr};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use tokio::sync::mpsc;

const PREFIX: &str = "pipewire:";

/// About 20 ms per read, close to what the cpal path delivers per callback.
const PERIOD_US: u32 = 20_000;

/// How long one wait for data may block, so stop is seen within this time.
const WAIT_MS: u32 = 100;

/// The node name in a `pipewire:<node>` device setting, or `None` for any
/// other device name.
pub fn node_name(device: &str) -> Option<&str> {
    let node = device.strip_prefix(PREFIX)?.trim();
    (!node.is_empty()).then_some(node)
}

/// The ALSA PCM name that opens the PipeWire plugin targeted at `node`.
fn alsa_pcm_name(node: &str) -> Result<String, AudioError> {
    if node.contains(['"', '\\']) {
        return Err(AudioError::DeviceNotFound(format!("{PREFIX}{node}")));
    }
    Ok(format!("pipewire:NODE=\"{node}\""))
}

/// Open a mono f32 capture PCM on `node`. PipeWire converts the node's format,
/// channels and rate, so the stream normally arrives at `sample_rate` already.
/// Returns the PCM, its actual rate, and its period in frames.
fn open(node: &str, sample_rate: u32) -> Result<(PCM, u32, usize), AudioError> {
    let fail =
        |e: alsa::Error| AudioError::Connection(format!("Cannot open PipeWire node '{node}': {e}"));
    let pcm = PCM::new(&alsa_pcm_name(node)?, Direction::Capture, false).map_err(fail)?;
    {
        let hwp = HwParams::any(&pcm).map_err(fail)?;
        hwp.set_access(Access::RWInterleaved).map_err(fail)?;
        hwp.set_format(Format::float()).map_err(fail)?;
        hwp.set_channels(1).map_err(fail)?;
        hwp.set_rate_near(sample_rate, ValueOr::Nearest)
            .map_err(fail)?;
        hwp.set_period_time_near(PERIOD_US, ValueOr::Nearest)
            .map_err(fail)?;
        pcm.hw_params(&hwp).map_err(fail)?;
    }
    let (rate, period) = {
        let hwp = pcm.hw_params_current().map_err(fail)?;
        let rate = hwp.get_rate().map_err(fail)?;
        let period = hwp.get_period_size().map_err(fail)?;
        (rate, period.max(1) as usize)
    };
    Ok((pcm, rate, period))
}

/// Start capturing from `node`. Returns the command sender and thread for
/// [`super::cpal_capture::CpalCapture`] to hold, and the chunk receiver.
#[allow(clippy::type_complexity)]
pub(super) fn start(
    node: &str,
    target_sample_rate: u32,
) -> Result<
    (
        std::sync::mpsc::Sender<CaptureCommand>,
        thread::JoinHandle<()>,
        mpsc::Receiver<Vec<f32>>,
    ),
    AudioError,
> {
    let (pcm, rate, period) = open(node, target_sample_rate)?;
    tracing::info!("Using audio device: PipeWire node {node}");
    tracing::debug!("PipeWire node stream: {rate} Hz, 1 channel, {period} frames per read");

    let resampler = Arc::new(Mutex::new(if rate == target_sample_rate {
        None
    } else {
        match StreamResampler::new(rate, target_sample_rate) {
            Ok(r) => Some(r),
            Err(e) => {
                tracing::error!("{e}; capture will run without rate conversion");
                None
            }
        }
    }));
    let samples = Arc::new(Mutex::new(Vec::<f32>::new()));
    let stream_dead = Arc::new(AtomicBool::new(false));
    let stop = Arc::new(AtomicBool::new(false));
    let (chunk_tx, chunk_rx) = mpsc::channel(64);
    let (cmd_tx, cmd_rx) = std::sync::mpsc::channel::<CaptureCommand>();

    let reader = {
        let samples = samples.clone();
        let resampler = resampler.clone();
        let stream_dead = stream_dead.clone();
        let stop = stop.clone();
        thread::spawn(move || {
            let io = match pcm.io_f32() {
                Ok(io) => io,
                Err(e) => {
                    tracing::error!("PipeWire node stream: {e}");
                    stream_dead.store(true, Ordering::SeqCst);
                    return;
                }
            };
            // Start explicitly: a capture PCM starts on its first read, and
            // the wait below comes before any read.
            if let Err(e) = pcm.start() {
                tracing::error!("Cannot start PipeWire node stream: {e}");
                stream_dead.store(true, Ordering::SeqCst);
                return;
            }
            let mut buf = vec![0f32; period];
            while !stop.load(Ordering::SeqCst) {
                // Wait with a timeout before reading: a stream that never gets
                // linked delivers nothing, and a bare read would block stop.
                match pcm.wait(Some(WAIT_MS)) {
                    Ok(false) => continue,
                    Ok(true) => {}
                    Err(e) => {
                        if let Err(e) = pcm.try_recover(e, true) {
                            tracing::error!("Audio stream error, capture is now dead: {e}");
                            stream_dead.store(true, Ordering::SeqCst);
                            break;
                        }
                        continue;
                    }
                }
                match io.readi(&mut buf) {
                    Ok(0) => {}
                    Ok(n) => deliver(buf[..n].to_vec(), &resampler, &samples, &chunk_tx),
                    Err(e) => {
                        if let Err(e) = pcm.try_recover(e, true) {
                            tracing::error!("Audio stream error, capture is now dead: {e}");
                            stream_dead.store(true, Ordering::SeqCst);
                            break;
                        }
                    }
                }
            }
            let _ = pcm.drop();
        })
    };

    let label = format!("{PREFIX}{node}");
    let thread_handle = thread::spawn(move || {
        run_commands(
            cmd_rx,
            &samples,
            &resampler,
            &stream_dead,
            &label,
            move || {
                stop.store(true, Ordering::SeqCst);
                let _ = reader.join();
            },
        );
    });

    Ok((cmd_tx, thread_handle, chunk_rx))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_name_needs_the_prefix_and_a_name() {
        assert_eq!(
            node_name("pipewire:alsa_input.usb-046d_Logitech_StreamCam-02.analog-stereo"),
            Some("alsa_input.usb-046d_Logitech_StreamCam-02.analog-stereo")
        );
        assert_eq!(node_name("pipewire: mic "), Some("mic"));
        assert_eq!(node_name("pipewire"), None);
        assert_eq!(node_name("pipewire:"), None);
        assert_eq!(node_name("default"), None);
        assert_eq!(node_name("sysdefault:CARD=Yeti"), None);
    }

    #[test]
    fn alsa_name_quotes_the_node() {
        assert_eq!(
            alsa_pcm_name("alsa_input.usb-x.analog-stereo").unwrap(),
            "pipewire:NODE=\"alsa_input.usb-x.analog-stereo\""
        );
        assert!(alsa_pcm_name("bad\"name").is_err());
    }

    /// Records 3 s from a real node. Run on a machine with PipeWire:
    /// `VOXTYPE_TEST_PW_NODE=<node.name> cargo test --lib records_from_a_named_node -- --ignored --nocapture`
    #[tokio::test]
    #[ignore = "needs PipeWire and a node named in VOXTYPE_TEST_PW_NODE"]
    async fn records_from_a_named_node() {
        use crate::audio::AudioCapture;
        let node = std::env::var("VOXTYPE_TEST_PW_NODE").expect("VOXTYPE_TEST_PW_NODE");
        let config = crate::config::AudioConfig {
            device: format!("{PREFIX}{node}"),
            ..Default::default()
        };
        let mut capture = crate::audio::cpal_capture::CpalCapture::new(&config).unwrap();
        let mut rx = capture.start().await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(3)).await;
        let samples = capture.stop().await.unwrap();
        let mut streamed = 0;
        while let Ok(chunk) = rx.try_recv() {
            streamed += chunk.len();
        }
        let lead = samples
            .iter()
            .position(|s| s.abs() > 1e-4)
            .unwrap_or(samples.len());
        let zeros = samples.iter().filter(|s| **s == 0.0).count();
        let rms = (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt();
        let peak = samples.iter().fold(0f32, |m, s| m.max(s.abs()));
        println!(
            "samples {} streamed {} lead_zero_ms {:.0} zero_frac {:.3} rms {:.4} peak {:.4}",
            samples.len(),
            streamed,
            lead as f32 * 1000.0 / config.sample_rate as f32,
            zeros as f32 / samples.len() as f32,
            rms,
            peak
        );
        let expected = 3 * config.sample_rate as usize;
        assert!(samples.len() > expected * 9 / 10, "got {}", samples.len());
    }
}
