"""RVC v2 voice conversion sidecar for Kotoba.

Accepts a WAV file + model name, returns voice-converted WAV.
Runs on http://127.0.0.1:50022 by default.
"""

from __future__ import annotations

import io
import os
import tempfile
from pathlib import Path

import uvicorn
from fastapi import FastAPI, File, Query, UploadFile
from fastapi.responses import JSONResponse, StreamingResponse
from rvc_python.infer import RVCInference

app = FastAPI(title="kotoba-rvc-sidecar")

MODELS_DIR = Path(
    os.environ.get("RVC_MODELS_DIR", str(Path.home() / ".kotoba" / "models" / "rvc"))
)

# Lazy-initialized RVC engine
_rvc: RVCInference | None = None
_rvc_loaded_model: str | None = None


def _get_rvc() -> RVCInference:
    """Return a shared RVCInference instance, creating one on first call."""
    global _rvc
    if _rvc is None:
        device = os.environ.get("RVC_DEVICE", "cpu")
        _rvc = RVCInference(device=device)
    return _rvc


def _find_pth(model_dir: Path) -> Path | None:
    """Find the .pth file in a model directory."""
    pth_files = list(model_dir.glob("*.pth"))
    return pth_files[0] if pth_files else None


def _find_index(model_dir: Path) -> Path | None:
    """Find the .index file in a model directory."""
    idx_files = list(model_dir.glob("*.index"))
    return idx_files[0] if idx_files else None


@app.get("/version")
async def version() -> dict[str, str]:
    """Return sidecar version for health checks."""
    return {"version": "0.2.0"}


@app.get("/models")
async def list_models() -> dict[str, list[str]]:
    """List available RVC voice models."""
    if not MODELS_DIR.exists():
        return {"models": []}
    models = [
        d.name
        for d in MODELS_DIR.iterdir()
        if d.is_dir() and _find_pth(d) is not None
    ]
    return {"models": sorted(models)}


@app.post("/convert")
async def convert(
    file: UploadFile = File(...),
    model: str = Query(..., description="RVC model name"),
    pitch: int = Query(0, description="Pitch shift in semitones"),
) -> StreamingResponse | JSONResponse:
    """Convert voice in uploaded WAV using the specified RVC model."""
    model_dir = (MODELS_DIR / model).resolve()
    if not str(model_dir).startswith(str(MODELS_DIR.resolve())):
        return JSONResponse(
            status_code=400,
            content={"error": "invalid model name"},
        )

    pth_path = _find_pth(model_dir)
    if pth_path is None:
        return JSONResponse(
            status_code=404,
            content={"error": f"model not found: {model}"},
        )

    input_bytes = await file.read()

    rvc = _get_rvc()

    # Load model if different from currently loaded one
    global _rvc_loaded_model
    if _rvc_loaded_model != str(pth_path):
        rvc.load_model(str(pth_path))
        _rvc_loaded_model = str(pth_path)

    # Set index file if available
    idx_path = _find_index(model_dir)
    if idx_path is not None:
        rvc.set_params(index_path=str(idx_path), index_rate=0.75)

    # Set pitch
    if pitch != 0:
        rvc.set_params(f0up_key=pitch)

    # Use temp files for I/O (rvc-python operates on file paths)
    with tempfile.NamedTemporaryFile(suffix=".wav", delete=False) as tmp_in:
        tmp_in.write(input_bytes)
        tmp_in_path = tmp_in.name

    tmp_out_path = tmp_in_path.replace(".wav", "_out.wav")

    try:
        rvc.infer_file(tmp_in_path, tmp_out_path)
        output_bytes = Path(tmp_out_path).read_bytes()
    finally:
        Path(tmp_in_path).unlink(missing_ok=True)
        Path(tmp_out_path).unlink(missing_ok=True)

    output_buf = io.BytesIO(output_bytes)
    output_buf.seek(0)

    return StreamingResponse(output_buf, media_type="audio/wav")


if __name__ == "__main__":
    port = int(os.environ.get("RVC_PORT", "50022"))
    uvicorn.run(app, host="127.0.0.1", port=port)
