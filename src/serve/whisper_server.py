"""Minimal OpenAI-compatible Whisper ASR server managed by kotoba."""
import sys, io, tempfile, uvicorn
from fastapi import FastAPI, UploadFile, File, Form
from faster_whisper import WhisperModel

app = FastAPI()
model = None

@app.on_event("startup")
def load():
    global model
    model_size = sys.argv[1] if len(sys.argv) > 1 else "large-v3-turbo"
    print(f"Loading Whisper model ({model_size})...", flush=True)
    model = WhisperModel(model_size, device="cpu", compute_type="float32")
    print("Whisper ready.", flush=True)

@app.post("/v1/audio/transcriptions")
async def transcribe(file: UploadFile = File(...), model_name: str = Form("whisper-1", alias="model")):
    data = await file.read()
    with tempfile.NamedTemporaryFile(suffix=".wav", delete=True) as f:
        f.write(data)
        f.flush()
        segments, info = model.transcribe(f.name, beam_size=5, language="ja",
                                           vad_filter=True,
                                           vad_parameters=dict(min_silence_duration_ms=500))
        text = "".join(s.text for s in segments).strip()
    return {"text": text}

if __name__ == "__main__":
    port = int(sys.argv[2]) if len(sys.argv) > 2 else 8000
    uvicorn.run(app, host="127.0.0.1", port=port, log_level="warning")
