# Architecture: `serve` module

The `serve` module exposes kotoba's TTS pipeline as an OpenAI-compatible HTTP/WebSocket
API. This document describes the physical layout, key design decisions, and the
test architecture.

## Physical layout

```
src/serve/
├── mod.rs       # Router setup, run() entry point, build_router() for tests
├── handlers.rs  # Route handlers + BackendFactory trait + sentence splitting
├── models.rs    # Request/response types (SpeechRequest, WsClientMessage, WsResponse, ...)
└── tests.rs     # Unit + integration tests (24 tests)
```

Total: ~1000 lines including tests. The module is self-contained — it depends on
`crate::tts`, `crate::rvc`, `crate::app_config`, and `crate::paths`, but nothing
in the rest of the codebase depends on `serve`.

## Endpoints

| Path | Method | Purpose |
|------|--------|---------|
| `/v1/audio/speech` | POST | OpenAI-compatible HTTP batch synthesis |
| `/ws/tts` | WS | Sentence-level streaming with cancellation |
| `/v1/voices` | GET | List available voices |
| `/health` | GET | Health check |

## Key design decisions

### 1. `BackendFactory` trait — testability through dependency injection

**Problem**: handlers originally instantiated `KokoroBackend::new(...)` inline.
This made integration tests impossible without real ONNX models on disk.

**Solution**: extract a `BackendFactory` trait. `AppState` holds an
`Arc<dyn BackendFactory>`. Production uses `DefaultBackendFactory`, which
preserves the original match-on-name behavior. Tests inject a `StubBackendFactory`
that returns a stub backend writing minimal valid WAV files.

```rust
#[async_trait]
pub trait BackendFactory: Send + Sync {
    fn create(&self, backend: &str, speaker: &str, speed: f32)
        -> Result<Box<dyn TtsBackend>>;
}
```

The full handler pipeline (validation, voice resolution, file I/O, response framing)
is exercised end-to-end without touching ONNX or Python.

### 2. Sentence-level WebSocket streaming, not audio-level

**Problem**: rara#1212's Mini App needs low first-chunk latency for real-time
voice conversation. Kokoro is a batch TTS — the whole utterance is synthesized
before any audio is returned. We can't get true PCM-streaming from Kokoro.

**Solution**: split input text on Japanese sentence boundaries (`。！？\n`),
synthesize each sentence independently, and push each sentence's audio as a
separate binary WebSocket frame. First-chunk latency drops from "full text
synthesis" to "first sentence synthesis" (~2s instead of ~5-10s for long text).

This is text-level chunking, not audio-level streaming. The trade-off: no
sub-sentence streaming, but no engine changes either. Future Qwen3-TTS MLX
backend can add true PCM streaming as a separate concern.

### 3. Cancellation between chunks, not mid-synthesis

**Problem**: when a user starts speaking, the server should stop generating
TTS. But polling for cancel during ONNX inference is impractical.

**Solution**: between sentences, do a non-blocking `recv` check on the
WebSocket (`tokio::time::timeout(Duration::ZERO, ...)`). If a cancel message
is queued, abort remaining sentences and send `{"type": "cancelled"}`.

The trade-off: cancel latency is bounded by current sentence's synthesis time
(~1-3s). This is acceptable because individual sentences are short. Avoiding
the alternative — splitting the socket into separate read/write halves and
managing concurrent tasks — keeps the handler significantly simpler.

### 4. Backwards-compatible message parsing

`WsClientMessage::parse` tries the tagged format first, then falls back to the
legacy untagged `WsTtsRequest`. Existing clients continue to work; new clients
can use the tagged format for cancel support.

```rust
// Tagged (new):
{"type": "tts", "text": "...", "voice": "..."}
{"type": "cancel"}

// Untagged (legacy):
{"text": "...", "voice": "..."}
```

### 5. Voice routing — three-step resolution

```
voice="kokoro:jf_alpha"  → ("kokoro", "jf_alpha", None)
voice="hanazawa-kana"    → ("kokoro", default_voice, Some("hanazawa-kana"))  [if RVC dir exists]
voice="jf_alpha"         → ("kokoro", "jf_alpha", None)  [bare Kokoro voice fallback]
voice="garbage"          → 400 error
```

This is intentionally permissive on the input format because rara and other
clients use different conventions. The resolution function is pure and unit-tested.

## Test architecture

### What we test (matklad's "test features, not code")

| Layer | Examples |
|-------|----------|
| Pure functions | `split_sentences`, `WsClientMessage::parse`, `WsResponse` serialization |
| HTTP contracts | `POST /v1/audio/speech` returns WAV bytes; 400 on bad input |
| WebSocket contracts | Multi-sentence input → N binary frames + chunk msgs + done(N) |

We don't test handler internals, error message wording, or implementation details
that would lock the design in place.

### How we test it

**Pure unit tests** live in `handlers.rs` under `#[cfg(test)] mod tests`. They
don't need any state, server, or async runtime.

**Integration tests** live in `serve/tests.rs`. They start a real axum server on
a random port (port 0) using a stub `BackendFactory`, then exercise the full
HTTP/WebSocket pipeline:

```rust
async fn test_server() -> (String, JoinHandle<()>) {
    let state = AppState {
        config: Arc::new(AppConfig::default()),
        factory: Arc::new(StubBackendFactory),
    };
    let app = build_router(state);
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let handle = tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (url, handle)
}
```

WebSocket tests use `tokio-tungstenite` as a real client. HTTP tests use `reqwest`.

### Why a real server, not `oneshot`?

axum's `tower::ServiceExt::oneshot` works for HTTP but not for WebSocket upgrades —
the upgrade handshake requires actual TCP I/O. Rather than mixing two test
strategies, we use a real server for both.

The cost: each test binds a TCP port. The benefit: tests exercise exactly the
code path that production uses, including the upgrade handshake.

## Adding a new endpoint

1. Add the route in `mod.rs::build_router`
2. Add the handler function in `handlers.rs`
3. Add request/response types in `models.rs`
4. Add tests in `tests.rs`:
   - At least one happy-path integration test
   - One test per documented error case

The `BackendFactory` indirection means new endpoints get test coverage for free —
no need to mock anything beyond the factory.
