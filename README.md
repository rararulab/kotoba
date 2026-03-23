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
kotoba add 成功 --level n5               # Add vocabulary (auto-fill reading + meaning)
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
kotoba add 協力 --level n5               # Add with auto-filled reading + meaning
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
| `kokoro+rvc` | `kokoro:<voice>+rvc:<model>` | Kokoro model + RVC model + Python venv (see below) |

## RVC Voice Conversion Setup

RVC (Retrieval-based Voice Conversion) lets you transform Kokoro TTS output into anime character voices.

### 1. Create Python environment

```bash
uv venv --python 3.10 ~/.kotoba/venvs/rvc
uv pip install --python ~/.kotoba/venvs/rvc/bin/python3 \
  infer-rvc-python soundfile "setuptools<81" "numpy<2"
```

### 2. Download models

```bash
kotoba huggingface add kokoro                              # Base TTS model
kotoba huggingface add rvc:user/model                      # RVC model from HuggingFace
kotoba huggingface add rvc:ttttdiva/rvc_okiba:Hatsune_Miku # Multi-model repo with subpath
```

### 3. Set voice and test

```bash
kotoba voice set kokoro:af_heart+rvc:Hatsune_Miku
kotoba play こんにちは
```

### Custom Python path

If your Python is not at `~/.kotoba/venvs/rvc/bin/python3`:

```bash
export RVC_PYTHON=/path/to/python3               # env var (temporary)
kotoba config set rvc.python /path/to/python3     # config (permanent)
```

Run `kotoba doctor` to verify setup.

## License

MIT
