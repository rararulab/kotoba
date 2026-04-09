# kotoba

Immersive Japanese language learning CLI — Rust + SQLite, SRS-powered vocabulary and grammar tracking with multi-backend TTS (VOICEVOX, Kokoro ONNX, VITS) and optional RVC voice conversion.

## Install

```bash
git clone https://github.com/rararulab/kotoba && cd kotoba && cargo install --path .
```

## Documentation

- Chinese quick guide: [docs/usage.zh-CN.md](docs/usage.zh-CN.md)
- TTS API server architecture: [docs/architecture/serve.md](docs/architecture/serve.md)

## Quick Start

```bash
kotoba setup                             # Download VOICEVOX, init DB, configure default Kokoro+RVC voice
kotoba add 成功 --level n5               # Add vocabulary (auto-fill reading + meaning)
kotoba review                            # Show due reviews
kotoba seen 成功 recalled                 # Record review (forgot/recognized/recalled)
kotoba play 成功                          # Pronounce with TTS
kotoba play 成功 --enable                 # Pronounce and play immediately
kotoba play "今日は本当に嬉しい！" --style dramatic --enable
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
kotoba play 成功 --enable                 # Play via local output device immediately
kotoba play "今日は本当に嬉しい！" --style character  # Character-like expressive delivery
kotoba play "こんにちは" --style neutral   # Flatter, more stable delivery
kotoba play "こんにちは" --style soft      # Gentler and slower delivery
kotoba voice tone list                   # List tone presets
kotoba voice tone set balanced           # Apply a tone preset
kotoba voice list                        # List available voices
kotoba voice set voicevox:3              # Use VOICEVOX speaker 3
kotoba voice set kokoro:af_heart         # Use Kokoro ONNX (local, no server needed)
kotoba voice set kokoro:af_heart+rvc:naruto  # Kokoro + RVC voice conversion
kotoba config set voice.speed 0.85       # Slow down speech speed
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
kotoba setup                             # Full setup (VOICEVOX + DB + default Kokoro+RVC voice)
kotoba doctor                            # Health check all dependencies
kotoba doctor --json                     # Machine-readable health report
kotoba config set voice.active kokoro:af_heart  # Set config values
kotoba serve --port 3000                 # Start OpenAI-compatible TTS API server
```

## TTS API Server (`kotoba serve`)

Exposes kotoba's TTS pipeline (Kokoro/VOICEVOX/VITS + RVC) over HTTP and WebSocket
for use as a drop-in TTS backend for OpenAI-compatible clients (e.g. rara).

```bash
kotoba serve --host 127.0.0.1 --port 3000
```

### Endpoints

| Endpoint | Protocol | Purpose |
|----------|----------|---------|
| `POST /v1/audio/speech` | HTTP | OpenAI-compatible batch synthesis |
| `WS /ws/tts` | WebSocket | Sentence-level streaming with cancellation |
| `GET /v1/voices` | HTTP | List available voices |
| `GET /health` | HTTP | Health check |
| `GET /demo` | HTTP | Bundled web demo for streaming TTS |

After starting the server, open `http://localhost:3000/demo` in a browser
to try streaming TTS interactively — no build step or extra hosting required.

### HTTP batch synthesis

```bash
curl -X POST http://localhost:3000/v1/audio/speech \
  -H 'Content-Type: application/json' \
  -d '{"input":"こんにちは","voice":"kokoro:jf_alpha"}' \
  -o speech.wav
```

Request fields: `input` (required), `voice` (required), `model`, `response_format`, `speed`.

### WebSocket streaming

The `/ws/tts` endpoint splits input on Japanese sentence boundaries (`。！？\n`)
and streams each sentence's audio as a separate binary frame, optimizing
first-chunk latency.

```
Client → {"text": "長い文。複数の文。", "voice": "kokoro:jf_alpha"}
Server → [binary: WAV for sentence 1]
Server → {"type": "chunk", "index": 0}
Server → [binary: WAV for sentence 2]
Server → {"type": "chunk", "index": 1}
Server → {"type": "done", "chunks": 2}
```

Cancel mid-stream:
```
Client → {"type": "cancel"}
Server → {"type": "cancelled"}
```

Both tagged (`{"type":"tts",...}`) and legacy untagged (`{"text":...}`) request
formats are supported.

### Voice routing

The `voice` field is resolved in this order:
1. `backend:speaker_id` (e.g. `kokoro:jf_alpha`, `voicevox:3`) — direct TTS
2. RVC model name (e.g. `hanazawa-kana`) — Kokoro TTS + RVC conversion
3. Bare Kokoro voice name (e.g. `jf_alpha`) — Kokoro TTS

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

`play --style` supports `neutral`, `character` (default), `soft`, `dramatic`, `energetic`.

## License

MIT
