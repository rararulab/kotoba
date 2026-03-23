//! `CosyVoice` TTS backend via HTTP runtime service.

use std::path::Path;

use async_trait::async_trait;
use snafu::{ResultExt, ensure};

use super::TtsBackend;
use crate::error::{self, Result};

const COSYVOICE_SAMPLE_RATE: u32 = 22_050;
const COSYVOICE_CHANNELS: u16 = 1;

#[derive(Debug, Clone, Copy)]
enum CosyvoiceMode {
    Sft,
    ZeroShot,
    CrossLingual,
    Instruct,
}

impl CosyvoiceMode {
    fn parse(mode: &str) -> Option<Self> {
        let normalized = mode.trim().to_ascii_lowercase();
        match normalized.as_str() {
            "sft" => Some(Self::Sft),
            "zero_shot" | "zero-shot" | "zeroshot" => Some(Self::ZeroShot),
            "cross_lingual" | "cross-lingual" | "crosslingual" => Some(Self::CrossLingual),
            "instruct" => Some(Self::Instruct),
            _ => None,
        }
    }

    const fn endpoint(self) -> &'static str {
        match self {
            Self::Sft => "inference_sft",
            Self::ZeroShot => "inference_zero_shot",
            Self::CrossLingual => "inference_cross_lingual",
            Self::Instruct => "inference_instruct",
        }
    }
}

/// `CosyVoice` backend configuration bound to one synthesis request profile.
pub struct CosyvoiceBackend {
    base_url:      String,
    mode:          String,
    speaker_id:    String,
    prompt_text:   String,
    prompt_wav:    String,
    instruct_text: String,
}

impl CosyvoiceBackend {
    /// Create a new `CosyVoice` backend.
    pub const fn new(
        base_url: String,
        mode: String,
        speaker_id: String,
        prompt_text: String,
        prompt_wav: String,
        instruct_text: String,
    ) -> Self {
        Self {
            base_url,
            mode,
            speaker_id,
            prompt_text,
            prompt_wav,
            instruct_text,
        }
    }

    fn resolve_mode(&self) -> Result<CosyvoiceMode> {
        CosyvoiceMode::parse(&self.mode).ok_or_else(|| {
            error::CosyvoiceSnafu {
                message: format!(
                    "unsupported cosyvoice.mode '{}'; expected one of: sft, zero_shot, \
                     cross_lingual, instruct",
                    self.mode
                ),
            }
            .build()
        })
    }

    fn endpoint_url(&self, mode: CosyvoiceMode) -> String {
        let base = self.base_url.trim_end_matches('/');
        format!("{base}/{}", mode.endpoint())
    }

    fn prompt_wav_part(&self) -> Result<reqwest::multipart::Part> {
        ensure!(
            !self.prompt_wav.trim().is_empty(),
            error::CosyvoiceSnafu {
                message: "cosyvoice.prompt_wav is required for zero_shot/cross_lingual mode"
                    .to_string(),
            }
        );

        let wav_path = Path::new(self.prompt_wav.trim());
        let file_name = wav_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("prompt.wav")
            .to_string();
        let content = std::fs::read(wav_path).context(error::IoSnafu)?;
        reqwest::multipart::Part::bytes(content)
            .file_name(file_name)
            .mime_str("application/octet-stream")
            .map_err(|source| {
                error::CosyvoiceSnafu {
                    message: format!("failed to build prompt_wav multipart part: {source}"),
                }
                .build()
            })
    }
}

#[async_trait]
impl TtsBackend for CosyvoiceBackend {
    async fn synthesize(&self, text: &str, output: &Path) -> Result<()> {
        let mode = self.resolve_mode()?;
        let url = self.endpoint_url(mode);
        let client = crate::http::client();

        let response = match mode {
            CosyvoiceMode::Sft => client
                .get(&url)
                .query(&[("tts_text", text), ("spk_id", self.speaker_id.as_str())])
                .send()
                .await
                .context(error::HttpSnafu)?,
            CosyvoiceMode::ZeroShot => {
                ensure!(
                    !self.prompt_text.trim().is_empty(),
                    error::CosyvoiceSnafu {
                        message: "cosyvoice.prompt_text is required for zero_shot mode".to_string(),
                    }
                );
                let form = reqwest::multipart::Form::new()
                    .text("tts_text", text.to_string())
                    .text("prompt_text", self.prompt_text.clone())
                    .part("prompt_wav", self.prompt_wav_part()?);
                client
                    .get(&url)
                    .multipart(form)
                    .send()
                    .await
                    .context(error::HttpSnafu)?
            }
            CosyvoiceMode::CrossLingual => {
                let form = reqwest::multipart::Form::new()
                    .text("tts_text", text.to_string())
                    .part("prompt_wav", self.prompt_wav_part()?);
                client
                    .get(&url)
                    .multipart(form)
                    .send()
                    .await
                    .context(error::HttpSnafu)?
            }
            CosyvoiceMode::Instruct => {
                ensure!(
                    !self.instruct_text.trim().is_empty(),
                    error::CosyvoiceSnafu {
                        message: "cosyvoice.instruct_text is required for instruct mode"
                            .to_string(),
                    }
                );
                client
                    .get(&url)
                    .query(&[
                        ("tts_text", text),
                        ("spk_id", self.speaker_id.as_str()),
                        ("instruct_text", self.instruct_text.as_str()),
                    ])
                    .send()
                    .await
                    .context(error::HttpSnafu)?
            }
        };

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.context(error::HttpSnafu)?;
            return error::CosyvoiceSnafu {
                message: format!(
                    "request failed ({} {}): {}",
                    status.as_u16(),
                    status,
                    body.chars().take(300).collect::<String>()
                ),
            }
            .fail();
        }

        let audio = response.bytes().await.context(error::HttpSnafu)?;
        if audio.starts_with(b"RIFF") {
            std::fs::write(output, &audio).context(error::IoSnafu)?;
            return Ok(());
        }

        let wrapped = pcm16le_to_wav_bytes(&audio, COSYVOICE_SAMPLE_RATE, COSYVOICE_CHANNELS)?;
        std::fs::write(output, wrapped).context(error::IoSnafu)?;
        Ok(())
    }
}

fn pcm16le_to_wav_bytes(pcm: &[u8], sample_rate: u32, channels: u16) -> Result<Vec<u8>> {
    ensure!(
        pcm.len().is_multiple_of(2),
        error::CosyvoiceSnafu {
            message: "cosyvoice returned odd-length PCM payload".to_string(),
        }
    );

    let data_size: u32 = pcm.len().try_into().map_err(|_| {
        error::CosyvoiceSnafu {
            message: "cosyvoice payload is too large to encode as WAV".to_string(),
        }
        .build()
    })?;
    let bits_per_sample: u16 = 16;
    let block_align = channels * (bits_per_sample / 8);
    let byte_rate = sample_rate * u32::from(block_align);
    let riff_size = 36u32.checked_add(data_size).ok_or_else(|| {
        error::CosyvoiceSnafu {
            message: "cosyvoice payload overflow when building WAV header".to_string(),
        }
        .build()
    })?;

    let mut wav = Vec::with_capacity(44 + pcm.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&riff_size.to_le_bytes());
    wav.extend_from_slice(b"WAVE");
    wav.extend_from_slice(b"fmt ");
    wav.extend_from_slice(&16u32.to_le_bytes());
    wav.extend_from_slice(&1u16.to_le_bytes());
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&bits_per_sample.to_le_bytes());
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&data_size.to_le_bytes());
    wav.extend_from_slice(pcm);
    Ok(wav)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_mode_accepts_aliases() {
        assert!(matches!(
            CosyvoiceMode::parse("zero_shot"),
            Some(CosyvoiceMode::ZeroShot)
        ));
        assert!(matches!(
            CosyvoiceMode::parse("cross-lingual"),
            Some(CosyvoiceMode::CrossLingual)
        ));
        assert!(matches!(
            CosyvoiceMode::parse("sft"),
            Some(CosyvoiceMode::Sft)
        ));
        assert!(CosyvoiceMode::parse("unknown").is_none());
    }

    #[test]
    fn pcm16_wrapper_builds_valid_wav_header() {
        let pcm = [0u8, 0u8, 255u8, 127u8];
        let wav = pcm16le_to_wav_bytes(&pcm, COSYVOICE_SAMPLE_RATE, COSYVOICE_CHANNELS)
            .expect("expected pcm conversion to succeed");
        assert!(wav.starts_with(b"RIFF"));
        assert_eq!(&wav[8..12], b"WAVE");
        assert_eq!(wav.len(), 44 + pcm.len());
    }

    #[test]
    fn pcm16_wrapper_rejects_odd_length() {
        let err = pcm16le_to_wav_bytes(&[1u8], COSYVOICE_SAMPLE_RATE, COSYVOICE_CHANNELS)
            .expect_err("odd-length payload should fail");
        let msg = err.to_string();
        assert!(msg.contains("odd-length PCM"));
    }
}
