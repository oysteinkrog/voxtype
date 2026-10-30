//! State machine for voxtype daemon
//!
//! Defines the states for the push-to-talk workflow:
//! Idle → Recording → Transcribing → Outputting → Idle

use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Audio samples collected during recording (f32, mono, 16kHz)
pub type AudioBuffer = Vec<f32>;

/// Result from transcribing a single chunk during eager processing
#[derive(Debug, Clone)]
pub struct ChunkResult {
    /// Transcribed text from this chunk
    pub text: String,
    /// Which chunk this result corresponds to (0-indexed)
    pub chunk_index: usize,
}

/// Where a recording is in the keyboard-edit flow. Without
/// `[osd] keyboard_edit` a recording stays in `Listening` throughout.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Phase {
    /// Audio flows to the transcriber, as in every recording today.
    #[default]
    Listening,
    /// A single edit-key tap ended; audio is held until it is confirmed
    /// or cancelled.
    Pending,
    /// The tap is confirmed; silence padding flushes the lookahead.
    Settling,
    /// The transcriber is paused and the popup owns the text.
    Editing,
    /// Focus left the popup without a request; still paused.
    Detached,
    /// Resume or finish was asked for; focus is being handed back.
    Releasing,
}

impl Phase {
    /// Name written for the popup next to the unchanged daemon state.
    pub fn as_str(self) -> &'static str {
        match self {
            Phase::Listening => "listening",
            Phase::Pending => "pending",
            Phase::Settling => "settling",
            Phase::Editing => "editing",
            Phase::Detached => "detached",
            Phase::Releasing => "releasing",
        }
    }

    /// Every phase but `Listening` stops the recording clock.
    pub fn is_paused(self) -> bool {
        self != Phase::Listening
    }
}

/// The edit phase of a recording and the time it has spent paused, so
/// `max_duration_secs` counts listening time only.
#[derive(Debug, Clone, Default)]
pub struct Pause {
    phase: Phase,
    since: Option<Instant>,
    total: Duration,
}

impl Pause {
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// Move to `phase` at `now`. Moving between two paused phases keeps
    /// the pause running from when it began.
    pub fn set(&mut self, phase: Phase, now: Instant) {
        match (self.since, phase.is_paused()) {
            (None, true) => self.since = Some(now),
            (Some(t), false) => {
                self.total += now.saturating_duration_since(t);
                self.since = None;
            }
            _ => {}
        }
        self.phase = phase;
    }

    /// Time spent out of `Listening` up to `now`.
    pub fn paused_for(&self, now: Instant) -> Duration {
        self.total
            + self
                .since
                .map_or(Duration::ZERO, |t| now.saturating_duration_since(t))
    }
}

/// Application state
#[derive(Debug, Clone)]
pub enum State {
    /// Waiting for hotkey press
    Idle,

    /// Hotkey held, recording audio
    Recording {
        /// When recording started
        started_at: Instant,
        /// Optional model override for this recording
        model_override: Option<String>,
        /// Edit phase and paused time
        pause: Pause,
    },

    /// Hotkey held, recording audio with eager chunk processing
    EagerRecording {
        /// When recording started
        started_at: Instant,
        /// Optional model override for this recording
        model_override: Option<String>,
        /// Accumulated audio samples during recording
        accumulated_audio: AudioBuffer,
        /// Number of chunks already sent for transcription
        chunks_sent: usize,
        /// Results received from completed chunk transcriptions
        chunk_results: Vec<ChunkResult>,
        /// Number of transcription tasks currently in flight
        tasks_in_flight: usize,
        /// Edit phase and paused time
        pause: Pause,
    },

    /// Hotkey released, transcribing audio
    Transcribing {
        /// Recorded audio samples
        audio: AudioBuffer,
    },

    /// Transcription complete, outputting text
    Outputting {
        /// Transcribed text
        text: String,
    },

    /// Hotkey held, audio is being streamed to a streaming-capable backend
    /// which is emitting partial and final segments concurrently. Replaces
    /// `Recording` *and* `Transcribing` for streaming sessions.
    Streaming {
        /// When the streaming session started
        started_at: Instant,
        /// Optional model override for this session
        model_override: Option<String>,
        /// Most recent partial text (for status / debug only; never typed)
        partial_buffer: String,
        /// Concatenated finalized segments committed so far
        finalized_text: String,
        /// Number of characters typed to the output sink. Used by the
        /// daemon's cancel path to send N backspaces and rewind text
        /// that was already delivered to the user.
        typed_chars: usize,
        /// When set, this session targets file output (`--file=path` or
        /// a `mode = "file"` config/profile), not the live cursor.
        /// Segments accumulate into `finalized_text` without typing —
        /// see `StreamingSession::commit_segment_silent` — and the
        /// daemon writes the accumulated text to this path once the
        /// backend reports `Ended`.
        file_output_path: Option<PathBuf>,
        /// Edit phase and paused time
        pause: Pause,
    },
}

impl State {
    /// Create a new idle state
    pub fn new() -> Self {
        State::Idle
    }

    /// Check if in idle state
    pub fn is_idle(&self) -> bool {
        matches!(self, State::Idle)
    }

    /// Check if in recording state (normal, eager, or streaming).
    ///
    /// `Streaming` reports `true` here so existing status-reporting and
    /// hotkey-release logic continues to work without per-callsite changes.
    pub fn is_recording(&self) -> bool {
        matches!(
            self,
            State::Recording { .. } | State::EagerRecording { .. } | State::Streaming { .. }
        )
    }

    /// Check if in eager recording state specifically
    pub fn is_eager_recording(&self) -> bool {
        matches!(self, State::EagerRecording { .. })
    }

    /// Check if in streaming state specifically
    pub fn is_streaming(&self) -> bool {
        matches!(self, State::Streaming { .. })
    }

    /// Get recording duration if currently recording (normal, eager, or
    /// streaming). Time spent paused for an edit does not count.
    pub fn recording_duration(&self) -> Option<Duration> {
        self.recording_duration_at(Instant::now())
    }

    /// `recording_duration` as of `now`.
    pub fn recording_duration_at(&self, now: Instant) -> Option<Duration> {
        let (started_at, pause) = self.clock()?;
        Some(
            now.saturating_duration_since(*started_at)
                .saturating_sub(pause.paused_for(now)),
        )
    }

    /// Edit phase of the recording, or `None` when not recording.
    pub fn phase(&self) -> Option<Phase> {
        self.clock().map(|(_, pause)| pause.phase())
    }

    /// Move the recording to `phase` at `now`. Returns false, and changes
    /// nothing, when not recording.
    pub fn set_phase(&mut self, phase: Phase, now: Instant) -> bool {
        match self {
            State::Recording { pause, .. }
            | State::EagerRecording { pause, .. }
            | State::Streaming { pause, .. } => {
                pause.set(phase, now);
                true
            }
            _ => false,
        }
    }

    fn clock(&self) -> Option<(&Instant, &Pause)> {
        match self {
            State::Recording {
                started_at, pause, ..
            }
            | State::EagerRecording {
                started_at, pause, ..
            }
            | State::Streaming {
                started_at, pause, ..
            } => Some((started_at, pause)),
            _ => None,
        }
    }

    /// Get the number of chunks sent for transcription (eager mode only)
    pub fn eager_chunks_sent(&self) -> Option<usize> {
        match self {
            State::EagerRecording { chunks_sent, .. } => Some(*chunks_sent),
            _ => None,
        }
    }

    /// Get the number of transcription tasks currently in flight (eager mode only)
    pub fn eager_tasks_in_flight(&self) -> Option<usize> {
        match self {
            State::EagerRecording {
                tasks_in_flight, ..
            } => Some(*tasks_in_flight),
            _ => None,
        }
    }
}

impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for State {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            State::Idle => write!(f, "Idle"),
            State::Recording { started_at, .. } => {
                write!(f, "Recording ({:.1}s)", started_at.elapsed().as_secs_f32())
            }
            State::EagerRecording {
                started_at,
                chunks_sent,
                tasks_in_flight,
                ..
            } => {
                write!(
                    f,
                    "Recording ({:.1}s, {} chunks, {} pending)",
                    started_at.elapsed().as_secs_f32(),
                    chunks_sent,
                    tasks_in_flight
                )
            }
            State::Transcribing { audio } => {
                let duration = audio.len() as f32 / 16000.0;
                write!(f, "Transcribing ({:.1}s of audio)", duration)
            }
            State::Outputting { text } => {
                // Use chars() to handle multi-byte UTF-8 characters
                let preview = if text.chars().count() > 20 {
                    format!("{}...", text.chars().take(20).collect::<String>())
                } else {
                    text.clone()
                };
                write!(f, "Outputting: {:?}", preview)
            }
            State::Streaming {
                started_at,
                finalized_text,
                typed_chars,
                ..
            } => {
                write!(
                    f,
                    "Streaming ({:.1}s, {} typed, {} committed chars)",
                    started_at.elapsed().as_secs_f32(),
                    typed_chars,
                    finalized_text.chars().count()
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new_state_is_idle() {
        let state = State::new();
        assert!(state.is_idle());
    }

    #[test]
    fn test_recording_state() {
        let state = State::Recording {
            started_at: Instant::now(),
            model_override: None,
            pause: Pause::default(),
        };
        assert!(state.is_recording());
        assert!(!state.is_idle());
        assert!(state.recording_duration().is_some());
    }

    #[test]
    fn test_idle_has_no_duration() {
        let state = State::Idle;
        assert!(state.recording_duration().is_none());
    }

    #[test]
    fn test_state_display() {
        let state = State::Idle;
        assert_eq!(format!("{}", state), "Idle");

        let state = State::Recording {
            started_at: Instant::now(),
            model_override: None,
            pause: Pause::default(),
        };
        assert!(format!("{}", state).starts_with("Recording"));
    }

    #[test]
    fn test_eager_recording_state() {
        let state = State::EagerRecording {
            started_at: Instant::now(),
            model_override: None,
            accumulated_audio: vec![],
            chunks_sent: 2,
            chunk_results: vec![],
            tasks_in_flight: 1,
            pause: Pause::default(),
        };
        assert!(state.is_recording());
        assert!(state.is_eager_recording());
        assert!(!state.is_idle());
        assert!(state.recording_duration().is_some());
        assert_eq!(state.eager_chunks_sent(), Some(2));
        assert_eq!(state.eager_tasks_in_flight(), Some(1));
    }

    #[test]
    fn test_regular_recording_not_eager() {
        let state = State::Recording {
            started_at: Instant::now(),
            model_override: None,
            pause: Pause::default(),
        };
        assert!(state.is_recording());
        assert!(!state.is_eager_recording());
        assert_eq!(state.eager_chunks_sent(), None);
        assert_eq!(state.eager_tasks_in_flight(), None);
    }

    #[test]
    fn test_streaming_state_is_recording() {
        let state = State::Streaming {
            started_at: Instant::now(),
            model_override: None,
            partial_buffer: String::new(),
            finalized_text: String::new(),
            typed_chars: 0,
            file_output_path: None,
            pause: Pause::default(),
        };
        assert!(state.is_recording());
        assert!(state.is_streaming());
        assert!(!state.is_idle());
        assert!(!state.is_eager_recording());
        assert!(state.recording_duration().is_some());
    }

    #[test]
    fn test_streaming_state_display() {
        let state = State::Streaming {
            started_at: Instant::now(),
            model_override: None,
            partial_buffer: "hel".into(),
            finalized_text: "hello".into(),
            typed_chars: 5,
            file_output_path: None,
            pause: Pause::default(),
        };
        let display = format!("{}", state);
        assert!(display.starts_with("Streaming"));
        assert!(display.contains("5 typed"));
    }

    #[test]
    fn test_recording_states_are_not_streaming() {
        let r = State::Recording {
            started_at: Instant::now(),
            model_override: None,
            pause: Pause::default(),
        };
        assert!(!r.is_streaming());
        let e = State::EagerRecording {
            started_at: Instant::now(),
            model_override: None,
            accumulated_audio: vec![],
            chunks_sent: 0,
            chunk_results: vec![],
            tasks_in_flight: 0,
            pause: Pause::default(),
        };
        assert!(!e.is_streaming());
    }

    #[test]
    fn test_eager_recording_display() {
        let state = State::EagerRecording {
            started_at: Instant::now(),
            model_override: None,
            accumulated_audio: vec![],
            chunks_sent: 3,
            chunk_results: vec![],
            tasks_in_flight: 2,
            pause: Pause::default(),
        };
        let display = format!("{}", state);
        assert!(display.contains("Recording"));
        assert!(display.contains("3 chunks"));
        assert!(display.contains("2 pending"));
    }

    fn recording_at(t0: Instant) -> State {
        State::Recording {
            started_at: t0,
            model_override: None,
            pause: Pause::default(),
        }
    }

    #[test]
    fn duration_without_pause_is_elapsed_time() {
        let t0 = Instant::now();
        let state = recording_at(t0);
        let now = t0 + Duration::from_secs(10);
        assert_eq!(
            state.recording_duration_at(now),
            Some(Duration::from_secs(10))
        );
        assert_eq!(state.phase(), Some(Phase::Listening));
    }

    #[test]
    fn paused_and_pending_time_is_excluded() {
        let t0 = Instant::now();
        let s = |n: u64| t0 + Duration::from_secs(n);
        let mut state = recording_at(t0);
        // 2 s listening, 1 s pending, 5 s settling and editing, 3 s listening
        assert!(state.set_phase(Phase::Pending, s(2)));
        assert!(state.set_phase(Phase::Settling, s(3)));
        assert!(state.set_phase(Phase::Editing, s(4)));
        // Frozen while paused
        assert_eq!(
            state.recording_duration_at(s(4)),
            Some(Duration::from_secs(2))
        );
        assert_eq!(
            state.recording_duration_at(s(7)),
            Some(Duration::from_secs(2))
        );
        assert!(state.set_phase(Phase::Releasing, s(7)));
        assert!(state.set_phase(Phase::Listening, s(8)));
        assert_eq!(
            state.recording_duration_at(s(11)),
            Some(Duration::from_secs(5))
        );
        assert_eq!(state.phase(), Some(Phase::Listening));
    }

    #[test]
    fn cancelled_tap_excludes_only_the_pending_time() {
        let t0 = Instant::now();
        let ms = |n: u64| t0 + Duration::from_millis(n);
        let mut state = recording_at(t0);
        state.set_phase(Phase::Pending, ms(1000));
        state.set_phase(Phase::Listening, ms(1300));
        state.set_phase(Phase::Pending, ms(2000));
        state.set_phase(Phase::Listening, ms(2300));
        assert_eq!(
            state.recording_duration_at(ms(3000)),
            Some(Duration::from_millis(2400))
        );
    }

    #[test]
    fn every_recording_variant_tracks_its_phase() {
        let t0 = Instant::now();
        let mut states = [
            recording_at(t0),
            State::EagerRecording {
                started_at: t0,
                model_override: None,
                accumulated_audio: vec![],
                chunks_sent: 0,
                chunk_results: vec![],
                tasks_in_flight: 0,
                pause: Pause::default(),
            },
            State::Streaming {
                started_at: t0,
                model_override: None,
                partial_buffer: String::new(),
                finalized_text: String::new(),
                typed_chars: 0,
                file_output_path: None,
                pause: Pause::default(),
            },
        ];
        for state in &mut states {
            assert!(state.set_phase(Phase::Detached, t0 + Duration::from_secs(1)));
            assert_eq!(state.phase(), Some(Phase::Detached));
            assert_eq!(
                state.recording_duration_at(t0 + Duration::from_secs(9)),
                Some(Duration::from_secs(1))
            );
        }
    }

    #[test]
    fn idle_has_no_phase() {
        let mut state = State::Idle;
        assert!(!state.set_phase(Phase::Editing, Instant::now()));
        assert_eq!(state.phase(), None);
    }

    #[test]
    fn phase_names_are_stable() {
        let names: Vec<_> = [
            Phase::Listening,
            Phase::Pending,
            Phase::Settling,
            Phase::Editing,
            Phase::Detached,
            Phase::Releasing,
        ]
        .iter()
        .map(|p| p.as_str())
        .collect();
        assert_eq!(
            names,
            [
                "listening",
                "pending",
                "settling",
                "editing",
                "detached",
                "releasing"
            ]
        );
        assert!(!Phase::Listening.is_paused());
        assert!(Phase::Pending.is_paused());
    }
}
