"""RVC v2 voice conversion sidecar for Kotoba.

Accepts a WAV file + model name, returns voice-converted WAV.
Runs on http://localhost:50022 by default.
"""

from __future__ import annotations

import io
import os
from pathlib import Path

import uvicorn
from fastapi import FastAPI, File, Query, UploadFile
from fastapi.responses import JSONResponse, StreamingResponse

app = FastAPI(title="kotoba-rvc-sidecar")

MODELS_DIR = Path(
    os.environ.get("RVC_MODELS_DIR", str(Path.home() / ".kotoba" / "models" / "rvc"))
)


@app.get("/version")
async def version() -> dict[str, str]:
    """Return sidecar version for health checks."""
    return {"version": "0.1.0"}


@app.get("/models")
async def list_models() -> dict[str, list[str]]:
    """List available RVC voice models."""
    if not MODELS_DIR.exists():
        return {"models": []}
    models = [
        d.name
        for d in MODELS_DIR.iterdir()
        if d.is_dir() and (d / "model.pth").exists()
    ]
    return {"models": sorted(models)}


@app.post("/convert")
async def convert(
    file: UploadFile = File(...),
    model: str = Query(..., description="RVC model name"),
    pitch: int = Query(0, description="Pitch shift in semitones"),
) -> StreamingResponse | JSONResponse:
    """Convert voice in uploaded WAV using the specified RVC model."""
    model_dir = MODELS_DIR / model
    model_path = model_dir / "model.pth"

    if not model_path.exists():
        return JSONResponse(
            status_code=404,
            content={"error": f"model not found: {model}"},
        )

    input_bytes = await file.read()

    # TODO: Wire actual RVC v2 inference here.
    # Pipeline: load model.pth + model.index -> extract f0 -> convert -> output
    # For now, passthrough to validate the API contract.
    output_buf = io.BytesIO(input_bytes)
    output_buf.seek(0)

    return StreamingResponse(output_buf, media_type="audio/wav")


if __name__ == "__main__":
    port = int(os.environ.get("RVC_PORT", "50022"))
    uvicorn.run(app, host="0.0.0.0", port=port)
