//! Real-time voice conversation pipeline over WebSocket.
//!
//! Implements an mlx-live-style server-side pipeline:
//! client PCM audio -> VAD -> ASR (Whisper) -> LLM (streaming) -> TTS (Kokoro)
//! -> client PCM audio.

use std::{sync::Arc, time::Duration};

use axum::extract::ws::{Message, WebSocket};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

use super::handlers::AppState;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Input sample rate expected from the client (16 kHz mono int16).
const INPUT_SAMPLE_RATE: u32 = 16_000;

/// Output sample rate advertised to the client (Kokoro native rate).
const OUTPUT_SAMPLE_RATE: u32 = 24_000;

/// RMS energy threshold for speech detection (float32 scale).
const VAD_THRESHOLD: f32 = 0.01;

/// Number of consecutive frames above threshold to start speech.
const VAD_SPEECH_START_FRAMES: usize = 3;

/// Number of consecutive frames below threshold to end speech (~1 s at 31.25
/// fps).
const VAD_SPEECH_END_FRAMES: usize = 30;

/// Minimum number of samples to consider valid speech (0.5 s at 16 kHz).
const VAD_MIN_SPEECH_SAMPLES: usize = INPUT_SAMPLE_RATE as usize / 2;

/// Maximum number of chat turns kept in context.
const MAX_HISTORY_TURNS: usize = 10;

/// Samples per VAD frame (matches worklet output chunk size: ceil(16000 /
/// 31.25) = 512).
const VAD_FRAME_SIZE: usize = 512;

// ---------------------------------------------------------------------------
// Protocol messages (server -> client)
// ---------------------------------------------------------------------------

/// JSON message sent from server to client over the WebSocket.
#[derive(Debug, Serialize)]
#[serde(tag = "type")]
#[allow(dead_code)]
enum ServerMessage {
    /// Sent on connect with sample rate info.
    #[serde(rename = "init")]
    Init {
        #[serde(rename = "outputSampleRate")]
        output_sample_rate: u32,
        #[serde(rename = "inputSampleRate")]
        input_sample_rate:  u32,
    },
    /// Pipeline phase change.
    #[serde(rename = "state")]
    State { phase: String },
    /// ASR transcription result.
    #[serde(rename = "asr")]
    Asr { text: String },
    /// Partial LLM response (accumulated so far).
    #[serde(rename = "llm_partial")]
    LlmPartial { text: String },
    /// The current generation was interrupted by new speech.
    #[serde(rename = "interrupt")]
    Interrupt,
    /// Generation complete.
    #[serde(rename = "done")]
    Done,
    /// An error occurred.
    #[serde(rename = "error")]
    Error { message: String },
}

// ---------------------------------------------------------------------------
// Query parameters
// ---------------------------------------------------------------------------

/// Configuration passed as query parameters on the WebSocket URL.
#[derive(Debug, Clone, Deserialize)]
pub struct VoiceParams {
    /// Whisper-compatible ASR endpoint URL.
    #[serde(default = "default_asr_url")]
    pub asr_url:       String,
    /// OpenAI-compatible LLM endpoint base URL.
    #[serde(default = "default_llm_url")]
    pub llm_url:       String,
    /// LLM API key (optional).
    #[serde(default)]
    pub llm_key:       String,
    /// LLM model name.
    #[serde(default = "default_llm_model")]
    pub llm_model:     String,
    /// TTS voice identifier.
    #[serde(default = "default_voice")]
    pub voice:         String,
    /// System prompt for the LLM.
    #[serde(default = "default_system_prompt")]
    pub system_prompt: String,
}

fn default_asr_url() -> String { "http://localhost:8000/v1/audio/transcriptions".to_string() }

fn default_llm_url() -> String { "http://localhost:11434/v1".to_string() }

fn default_llm_model() -> String { "qwen2.5:7b".to_string() }

fn default_voice() -> String { "kokoro:jf_alpha".to_string() }

fn default_system_prompt() -> String {
    "You are a friendly conversation partner. Reply in the same language as the user, and keep \
     responses short and natural."
        .to_string()
}

// ---------------------------------------------------------------------------
// VAD state machine
// ---------------------------------------------------------------------------

/// Simple RMS-energy voice activity detector.
struct Vad {
    /// Accumulated PCM samples for the current utterance.
    buffer:      Vec<i16>,
    /// Number of consecutive frames above the speech threshold.
    above_count: usize,
    /// Number of consecutive frames below the speech threshold.
    below_count: usize,
    /// Whether we are currently inside a speech region.
    in_speech:   bool,
}

impl Vad {
    const fn new() -> Self {
        Self {
            buffer:      Vec::new(),
            above_count: 0,
            below_count: 0,
            in_speech:   false,
        }
    }

    /// Feed a frame of int16 PCM samples and return `Some(audio)` when a
    /// complete utterance has been detected (speech start + speech end).
    fn feed(&mut self, samples: &[i16]) -> Option<Vec<i16>> {
        let rms = compute_rms_i16(samples);
        let is_loud = rms > VAD_THRESHOLD;

        if is_loud {
            self.above_count += 1;
            self.below_count = 0;
        } else {
            self.below_count += 1;
            self.above_count = 0;
        }

        if !self.in_speech && self.above_count >= VAD_SPEECH_START_FRAMES {
            self.in_speech = true;
            debug!("VAD: speech start");
        }

        if self.in_speech {
            self.buffer.extend_from_slice(samples);
        }

        if self.in_speech && self.below_count >= VAD_SPEECH_END_FRAMES {
            self.in_speech = false;
            debug!("VAD: speech end, {} samples", self.buffer.len());

            if self.buffer.len() >= VAD_MIN_SPEECH_SAMPLES {
                let utterance = std::mem::take(&mut self.buffer);
                self.reset_counts();
                return Some(utterance);
            }
            // Too short — discard.
            self.buffer.clear();
            self.reset_counts();
        }

        None
    }

    /// Check if speech is currently detected (for interruption logic).
    #[allow(dead_code)]
    const fn is_speaking(&self) -> bool { self.in_speech }

    /// Reset internal counters (but not the buffer).
    const fn reset_counts(&mut self) {
        self.above_count = 0;
        self.below_count = 0;
    }

    /// Discard all buffered audio and reset state.
    fn reset(&mut self) {
        self.buffer.clear();
        self.in_speech = false;
        self.reset_counts();
    }
}

/// Compute RMS energy of int16 samples, returning a value in float32 scale.
fn compute_rms_i16(samples: &[i16]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f64 = samples
        .iter()
        .map(|&s| {
            let f = f64::from(s) / f64::from(i16::MAX);
            f * f
        })
        .sum();
    #[allow(clippy::cast_precision_loss)]
    let mean = sum_sq / samples.len() as f64;
    #[allow(clippy::cast_possible_truncation)]
    let rms = mean.sqrt() as f32;
    rms
}

// ---------------------------------------------------------------------------
// WAV encoding / decoding helpers
// ---------------------------------------------------------------------------

/// Wrap raw PCM int16 samples in a minimal WAV header.
#[allow(clippy::cast_possible_truncation)]
fn encode_wav(samples: &[i16], sample_rate: u32, channels: u16) -> Vec<u8> {
    let data_len = samples.len() * 2;
    let file_len = 36 + data_len;
    let byte_rate = sample_rate * u32::from(channels) * 2;
    let block_align = channels * 2;

    let mut buf = Vec::with_capacity(44 + data_len);
    // RIFF header
    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&(file_len as u32).to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    // fmt chunk
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // chunk size
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    // data chunk
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&(data_len as u32).to_le_bytes());
    for &s in samples {
        buf.extend_from_slice(&s.to_le_bytes());
    }
    buf
}

/// Read a WAV file and return samples as float32 in [-1, 1].
fn wav_to_float32(wav_bytes: &[u8]) -> Result<Vec<f32>, String> {
    // Find "data" chunk
    let data_pos = wav_bytes
        .windows(4)
        .position(|w| w == b"data")
        .ok_or("no data chunk in WAV")?;
    let header_end = data_pos + 8; // skip "data" + 4-byte size
    if wav_bytes.len() < header_end {
        return Err("WAV too short".to_string());
    }

    // Read bits per sample from fmt chunk (byte 34-35)
    let bits_per_sample = if wav_bytes.len() >= 36 {
        u16::from_le_bytes([wav_bytes[34], wav_bytes[35]])
    } else {
        16
    };

    let data = &wav_bytes[header_end..];

    match bits_per_sample {
        16 => {
            let samples: Vec<f32> = data
                .chunks_exact(2)
                .map(|chunk| {
                    let s = i16::from_le_bytes([chunk[0], chunk[1]]);
                    f32::from(s) / 32768.0
                })
                .collect();
            Ok(samples)
        }
        32 => {
            // Check if it's float32 (format tag at byte 20-21)
            let format_tag = if wav_bytes.len() >= 22 {
                u16::from_le_bytes([wav_bytes[20], wav_bytes[21]])
            } else {
                1
            };
            if format_tag == 3 {
                // IEEE float
                let samples: Vec<f32> = data
                    .chunks_exact(4)
                    .map(|chunk| f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
                    .collect();
                Ok(samples)
            } else {
                // int32
                let samples: Vec<f32> = data
                    .chunks_exact(4)
                    .map(|chunk| {
                        let s = i32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]);
                        #[allow(clippy::cast_precision_loss)]
                        let f = s as f32 / 2_147_483_648.0;
                        f
                    })
                    .collect();
                Ok(samples)
            }
        }
        other => Err(format!("unsupported bits per sample: {other}")),
    }
}

// ---------------------------------------------------------------------------
// ASR (Whisper-compatible HTTP endpoint)
// ---------------------------------------------------------------------------

/// Send audio to a Whisper-compatible ASR endpoint and return the transcript.
async fn transcribe(
    client: &reqwest::Client,
    asr_url: &str,
    audio: &[i16],
    sample_rate: u32,
) -> Result<String, String> {
    let wav = encode_wav(audio, sample_rate, 1);

    let part = reqwest::multipart::Part::bytes(wav)
        .file_name("audio.wav")
        .mime_str("audio/wav")
        .map_err(|e| format!("mime error: {e}"))?;

    let form = reqwest::multipart::Form::new()
        .part("file", part)
        .text("model", "whisper-1");

    let resp = client
        .post(asr_url)
        .multipart(form)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| format!("ASR request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("ASR returned {status}: {body}"));
    }

    let json: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("ASR response parse failed: {e}"))?;

    json["text"]
        .as_str()
        .map(|s| s.trim().to_string())
        .ok_or_else(|| "ASR response missing 'text' field".to_string())
}

// ---------------------------------------------------------------------------
// LLM (OpenAI-compatible streaming endpoint)
// ---------------------------------------------------------------------------

/// Chat message for the LLM API.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct ChatMessage {
    role:    String,
    content: String,
}

/// Stream LLM completion, sending each sentence to the `sentence_tx` channel
/// as soon as a sentence boundary is detected. Returns the full response text.
async fn stream_llm(
    client: &reqwest::Client,
    params: &VoiceParams,
    history: &[ChatMessage],
    sentence_tx: &mpsc::Sender<String>,
    partial_tx: &mpsc::Sender<String>,
) -> Result<String, String> {
    let url = format!("{}/chat/completions", params.llm_url.trim_end_matches('/'));

    let body = serde_json::json!({
        "model": params.llm_model,
        "messages": history,
        "stream": true,
    });

    let mut req = client
        .post(&url)
        .json(&body)
        .timeout(Duration::from_secs(60));

    if !params.llm_key.is_empty() {
        req = req.header("Authorization", format!("Bearer {}", params.llm_key));
    }

    let resp = req
        .send()
        .await
        .map_err(|e| format!("LLM request failed: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        return Err(format!("LLM returned {status}: {body}"));
    }

    let mut full_text = String::new();
    let mut sentence_buf = String::new();
    let mut stream = resp.bytes_stream();

    // Buffer for incomplete SSE lines across chunk boundaries.
    let mut line_buf = String::new();

    while let Some(chunk_result) = stream.next().await {
        let chunk = chunk_result.map_err(|e| format!("LLM stream error: {e}"))?;
        let text = String::from_utf8_lossy(&chunk);
        line_buf.push_str(&text);

        // Process complete lines.
        while let Some(newline_pos) = line_buf.find('\n') {
            let line = line_buf[..newline_pos].trim().to_string();
            line_buf = line_buf[newline_pos + 1..].to_string();

            if line.is_empty() || !line.starts_with("data: ") {
                continue;
            }

            let data = &line[6..];
            if data == "[DONE]" {
                break;
            }

            if let Ok(json) = serde_json::from_str::<serde_json::Value>(data)
                && let Some(content) = json["choices"][0]["delta"]["content"].as_str()
            {
                full_text.push_str(content);
                sentence_buf.push_str(content);

                // Send partial text update.
                let _ = partial_tx.send(full_text.clone()).await;

                // Check for sentence boundaries and emit complete sentences.
                while let Some(boundary) = find_sentence_boundary(&sentence_buf) {
                    let sentence = sentence_buf[..=boundary].trim().to_string();
                    sentence_buf = sentence_buf[boundary + 1..].to_string();
                    if !sentence.is_empty() && sentence_tx.send(sentence).await.is_err() {
                        // Receiver dropped (interrupted).
                        return Ok(full_text);
                    }
                }
            }
        }
    }

    // Flush remaining text as a final sentence.
    let remaining = sentence_buf.trim().to_string();
    if !remaining.is_empty() {
        let _ = sentence_tx.send(remaining).await;
    }

    Ok(full_text)
}

/// Find the byte index of the first sentence-ending character.
fn find_sentence_boundary(text: &str) -> Option<usize> {
    text.char_indices()
        .find(|(_, ch)| matches!(ch, '。' | '！' | '？' | '!' | '?' | '\n'))
        .map(|(i, ch)| i + ch.len_utf8() - 1)
}

// ---------------------------------------------------------------------------
// TTS helper
// ---------------------------------------------------------------------------

/// Synthesize a sentence to a WAV file and return the audio as float32 samples.
async fn synthesize_sentence(
    state: &AppState,
    voice: &str,
    sentence: &str,
) -> Result<Vec<f32>, String> {
    let (backend_name, speaker_id) = voice.split_once(':').map_or_else(
        || ("kokoro".to_string(), voice.to_string()),
        |(b, s)| (b.to_string(), s.to_string()),
    );

    let tmp_dir = tempfile::tempdir().map_err(|e| format!("tempdir failed: {e}"))?;
    let output_path = tmp_dir.path().join("tts.wav");

    let backend = state
        .factory
        .create(&backend_name, &speaker_id, 1.0, &state.config)
        .map_err(|e| format!("backend creation failed: {e}"))?;

    backend
        .synthesize(sentence, &output_path)
        .await
        .map_err(|e| format!("TTS synthesis failed: {e}"))?;

    let wav_bytes = tokio::fs::read(&output_path)
        .await
        .map_err(|e| format!("failed to read TTS output: {e}"))?;

    wav_to_float32(&wav_bytes)
}

// ---------------------------------------------------------------------------
// WebSocket session
// ---------------------------------------------------------------------------

/// Handle a voice WebSocket connection.
///
/// This is the main entry point called by the axum route handler.
pub async fn handle_voice_ws(mut socket: WebSocket, state: AppState, params: VoiceParams) {
    // Send init message.
    let init = ServerMessage::Init {
        output_sample_rate: OUTPUT_SAMPLE_RATE,
        input_sample_rate:  INPUT_SAMPLE_RATE,
    };
    if send_json(&mut socket, &init).await.is_err() {
        return;
    }

    let client = reqwest::Client::new();
    let mut history: Vec<ChatMessage> = vec![ChatMessage {
        role:    "system".to_string(),
        content: params.system_prompt.clone(),
    }];
    let mut vad = Vad::new();

    // Split the socket into sender/receiver so we can use them concurrently.
    let (ws_sender, mut ws_receiver) = socket.split();
    let ws_sender = Arc::new(tokio::sync::Mutex::new(ws_sender));

    // Send idle state.
    let _ = send_json_via(
        &ws_sender,
        &ServerMessage::State {
            phase: "idle".to_string(),
        },
    )
    .await;

    loop {
        let msg = match ws_receiver.next().await {
            Some(Ok(msg)) => msg,
            Some(Err(e)) => {
                debug!("WS receive error: {e}");
                break;
            }
            None => break,
        };

        match msg {
            Message::Binary(data) => {
                // Interpret as int16 PCM samples.
                let samples: Vec<i16> = data
                    .chunks_exact(2)
                    .map(|c| i16::from_le_bytes([c[0], c[1]]))
                    .collect();

                // Feed frames to VAD.
                for frame in samples.chunks(VAD_FRAME_SIZE) {
                    if let Some(utterance) = vad.feed(frame) {
                        // We have a complete utterance — process it.
                        let sender = Arc::clone(&ws_sender);
                        let client_clone = client.clone();
                        let params_clone = params.clone();
                        let state_clone = state.clone();
                        let history_snapshot = history.clone();

                        // Process the utterance and collect results.
                        let result = process_utterance(
                            &sender,
                            &client_clone,
                            &params_clone,
                            &state_clone,
                            &history_snapshot,
                            &utterance,
                        )
                        .await;

                        match result {
                            Ok((user_text, assistant_text)) => {
                                history.push(ChatMessage {
                                    role:    "user".to_string(),
                                    content: user_text,
                                });
                                history.push(ChatMessage {
                                    role:    "assistant".to_string(),
                                    content: assistant_text,
                                });
                                // Trim history to last N turns (keep system prompt).
                                trim_history(&mut history);
                            }
                            Err(e) => {
                                warn!("utterance processing error: {e}");
                                let _ =
                                    send_json_via(&sender, &ServerMessage::Error { message: e })
                                        .await;
                            }
                        }

                        // Back to idle.
                        let _ = send_json_via(
                            &ws_sender,
                            &ServerMessage::State {
                                phase: "idle".to_string(),
                            },
                        )
                        .await;
                        vad.reset();
                    }
                }
            }
            Message::Close(_) => break,
            _ => {}
        }
    }
}

/// Process a single detected utterance through the full pipeline:
/// ASR -> LLM (streaming) -> TTS (per-sentence) -> audio playback.
///
/// Returns `(user_text, assistant_text)` on success.
async fn process_utterance(
    ws_sender: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    client: &reqwest::Client,
    params: &VoiceParams,
    state: &AppState,
    history: &[ChatMessage],
    audio: &[i16],
) -> Result<(String, String), String> {
    // Phase: generating (ASR).
    let _ = send_json_via(
        ws_sender,
        &ServerMessage::State {
            phase: "generating".to_string(),
        },
    )
    .await;

    // 1. ASR
    let transcript = transcribe(client, &params.asr_url, audio, INPUT_SAMPLE_RATE).await?;

    if transcript.is_empty() {
        return Err("empty transcript".to_string());
    }

    let _ = send_json_via(
        ws_sender,
        &ServerMessage::Asr {
            text: transcript.clone(),
        },
    )
    .await;

    // 2. Build messages for LLM.
    let mut messages = history.to_vec();
    messages.push(ChatMessage {
        role:    "user".to_string(),
        content: transcript.clone(),
    });

    // 3. Stream LLM and synthesize sentences as they arrive.
    let (sentence_tx, mut sentence_rx) = mpsc::channel::<String>(16);
    let (partial_tx, mut partial_rx) = mpsc::channel::<String>(64);

    let client_for_llm = client.clone();
    let params_for_llm = params.clone();
    let messages_for_llm = messages.clone();

    // Spawn LLM streaming task.
    let llm_handle = tokio::spawn(async move {
        stream_llm(
            &client_for_llm,
            &params_for_llm,
            &messages_for_llm,
            &sentence_tx,
            &partial_tx,
        )
        .await
    });

    // Forward partial LLM updates to the client.
    let ws_for_partial = Arc::clone(ws_sender);
    let partial_handle = tokio::spawn(async move {
        while let Some(text) = partial_rx.recv().await {
            let _ = send_json_via(&ws_for_partial, &ServerMessage::LlmPartial { text }).await;
        }
    });

    // Consume sentences and synthesize TTS.
    let mut full_assistant_text = String::new();
    while let Some(sentence) = sentence_rx.recv().await {
        full_assistant_text.push_str(&sentence);

        match synthesize_sentence(state, &params.voice, &sentence).await {
            Ok(samples) => {
                // Send float32 PCM as binary frame.
                let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
                let mut sender = ws_sender.lock().await;
                if sender.send(Message::Binary(bytes.into())).await.is_err() {
                    error!("failed to send TTS audio");
                    break;
                }
            }
            Err(e) => {
                warn!("TTS error for sentence: {e}");
                let _ = send_json_via(
                    ws_sender,
                    &ServerMessage::Error {
                        message: format!("TTS failed: {e}"),
                    },
                )
                .await;
            }
        }
    }

    // Wait for LLM to finish.
    if let Ok(Ok(full_text)) = llm_handle.await
        && full_assistant_text.is_empty()
    {
        full_assistant_text = full_text;
    }

    // Wait for partial forwarder to finish.
    let _ = partial_handle.await;

    // Send done.
    let _ = send_json_via(ws_sender, &ServerMessage::Done).await;

    Ok((transcript, full_assistant_text))
}

/// Send a JSON text frame over a mutex-wrapped split sink.
async fn send_json_via(
    sender: &Arc<tokio::sync::Mutex<futures_util::stream::SplitSink<WebSocket, Message>>>,
    msg: &ServerMessage,
) -> Result<(), ()> {
    let json = serde_json::to_string(msg).expect("ServerMessage serializes to JSON");
    let mut guard = sender.lock().await;
    guard.send(Message::Text(json.into())).await.map_err(|_| ())
}

/// Send a JSON text frame over a non-split WebSocket.
async fn send_json(socket: &mut WebSocket, msg: &ServerMessage) -> Result<(), ()> {
    let json = serde_json::to_string(msg).expect("ServerMessage serializes to JSON");
    socket
        .send(Message::Text(json.into()))
        .await
        .map_err(|_| ())
}

/// Keep only the system prompt + last `MAX_HISTORY_TURNS` user/assistant pairs.
fn trim_history(history: &mut Vec<ChatMessage>) {
    // First message is always the system prompt.
    let non_system = history.len().saturating_sub(1);
    let max_non_system = MAX_HISTORY_TURNS * 2; // 2 messages per turn
    if non_system > max_non_system {
        let remove_count = non_system - max_non_system;
        history.drain(1..=remove_count);
    }
}
