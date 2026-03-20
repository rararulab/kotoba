# kotoba

Immersive Japanese language learning CLI — Rust + SQLite, SRS-powered vocabulary and grammar tracking with VOICEVOX TTS.

## Install

```bash
git clone https://github.com/rararulab/kotoba && cd kotoba && cargo install --path .
```

## Usage

```bash
kotoba init                              # Initialize database
kotoba status                            # Current level, vocab count, due reviews
kotoba add 成功 せいこう 成功 --level N5    # Add vocabulary
kotoba seen 成功 5                        # Record review (quality: 1/3/5)
kotoba review                            # Due vocabulary list (JSON)
kotoba progress                          # Learning statistics
kotoba play 成功                          # TTS pronunciation (requires VOICEVOX)
kotoba export json                       # Export vocabulary (json/csv/anki)
```

## License

MIT
