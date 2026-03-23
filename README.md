# kotoba

Immersive Japanese language learning CLI — Rust + SQLite, SRS-powered vocabulary and grammar tracking with multi-backend TTS (VOICEVOX, Kokoro ONNX, VITS) and optional RVC voice conversion.

## Install

```bash
git clone https://github.com/rararulab/kotoba && cd kotoba && cargo install --path .
```

## Documentation

- Chinese quick guide: [docs/usage.zh-CN.md](docs/usage.zh-CN.md)

## Quick Start

```bash
kotoba setup                             # Download VOICEVOX, init DB, configure env
kotoba add 成功 せいこう success --level N5 # Add vocabulary
kotoba review                            # Show due reviews
kotoba seen 成功 recalled                 # Record review (forgot/recognized/recalled)
kotoba play 成功                          # Pronounce with TTS
```

## Commands

### Learning

```bash
kotoba init                              # Initialize database (without full setup)
kotoba status                            # Current level, vocab count, due reviews
kotoba add 成功 せいこう success --level N5 # Add vocabulary
kotoba seen 成功 recalled                 # Record review (forgot/recognized/recalled)
kotoba review                            # Due vocabulary list (JSON)
kotoba progress                          # Learning statistics
kotoba list vocab                        # List all vocabulary
kotoba export json                       # Export vocabulary (json/csv/anki)
```

### Grammar

```bash
kotoba grammar add ～ている "ongoing action" --level N5
kotoba grammar list
kotoba review --grammar                  # Due grammar reviews
kotoba seen --grammar ～ている recognized # Record grammar review
```

### Voice & TTS

```bash
kotoba play 成功                          # Pronounce with current voice
kotoba voice list                        # List available voices
kotoba voice set voicevox:3              # Use VOICEVOX speaker 3
kotoba voice set kokoro:af_heart         # Use Kokoro ONNX (local, no server needed)
kotoba voice set kokoro:af_heart+rvc:naruto  # Kokoro + RVC voice conversion
```

### Model Management

```bash
kotoba huggingface add kokoro            # Download Kokoro ONNX model (~300MB)
kotoba huggingface add rvc:user/model    # Download RVC model from HuggingFace
kotoba huggingface add user/vits-model   # Download VITS ONNX model
kotoba huggingface list                  # List downloaded models
```

### System

```bash
kotoba setup                             # Full setup (VOICEVOX + DB + config)
kotoba doctor                            # Health check all dependencies
kotoba config set voice.active kokoro:af_heart  # Set config values
```

## TTS Backends

| Backend | Format | Requires |
|---------|--------|----------|
| `voicevox` | `voicevox:<speaker_id>` | VOICEVOX Engine running (`kotoba setup`) |
| `kokoro` | `kokoro:<voice>` | Kokoro ONNX model (`kotoba huggingface add kokoro`) |
| `vits` | `vits:<model>` | VITS model (`kotoba huggingface add user/model`) |
| `kokoro+rvc` | `kokoro:<voice>+rvc:<model>` | Kokoro model + RVC model; Python sidecar auto-starts |

## License

MIT
