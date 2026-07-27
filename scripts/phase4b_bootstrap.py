from __future__ import annotations

import base64
import hashlib
from pathlib import Path

EXPECTED = {
    "good000.b64": "717d331e39a0d511f5c3d76773e79519ef544efda216e6b74ed52f9d11aa7c1f",
    "good001.b64": "b08559b815112c8ba4bc566c0171cd67898ca7d6a6aa75f84e742d37612241cd",
    "good002.b64": "b19ccba39f265420aba2bf861241c1dad56c9efe0b840c0fa619bab7b89bf84f",
    "good003.b64": "c35e65a0e7f4462344a134a2e5f4ec20070155dfdbbd694c78e213f0bc8a40a2",
    "good004.b64": "4185c1ef187f57cec697e7917c2c4ec906b0db1784dbb4a47f4043ac518d78ac",
    "good005.b64": "1839d48711dc33f36a6a899c52fc7f6c0db6ae6b639f7fcdd2d5a9cc91dbb029",
    "good006.b64": "6f603ed23bdc9b3f2be7f469b1e510ab83e4b7daef9ad59bfd36c8e16e9df97f",
    "good007.b64": "2f26494ce10f6d2ce4977981149c64ce6f10ab0f2077d3c09ec1238ec8feee51",
    "good008.b64": "f4a6d2b1970dbccea44e6e1ae2a473a3a3dcbed14fad79bf8f1684f9ff263ffe",
    "good009.b64": "ea870d20804dccf2e3974d2feed34d9e0cfa8996675866b3d9c2849cba369d1d",
    "good010.b64": "984cc181fffee457cc232559959dd6c08cc29c17166809a2f289b899289d9ca5",
    "good011.b64": "5ffba2499bc2b66e31909be38f47ab9e79e808fc70ea3c5b0d653ccaabdad719",
    "good012.b64": "455d003ea81eeb90991698b161d449d4e1e92285e500a0bf4c5a4cd1b6cb772b",
    "good013.b64": "7fa3764a22e296337965db411999be519ebfea640309c23a69ff94e90e17e467",
    "good014.b64": "5f430dd08d0d458d78a651b125d5c6977e2c3dc35bb1f6d7ac52e6583408cb3c",
    "good015.b64": "9c6b8b58293ac3f5c732a49c9ef742787b8f25bfefe26516376e9ed573b3c5c5",
    "good016.b64": "08630b977ddbdd6810e37dba208440801d490441b469ffa5bf022fcc53e9269d",
    "good017.b64": "d0cc36f828eac55cb27ac87ea0a7441833a61c10af1e5bf08bb38aab5c9e74a3",
    "good018.b64": "c118f8adfd132a35ae3af8294a2106d756620ddf47ba839eb36dc96add3abcd8",
}

parts_directory = Path(__file__).resolve().parent / "phase4b_good"
parts = sorted(parts_directory.glob("good*.b64"))
if [part.name for part in parts] != list(EXPECTED):
    raise RuntimeError("Phase 4B payload segment list is incomplete or out of order")

chunks: list[str] = []
for part in parts:
    text = part.read_text(encoding="utf-8")
    digest = hashlib.sha256(text.encode("utf-8")).hexdigest()
    if digest != EXPECTED[part.name]:
        raise RuntimeError(f"Phase 4B payload checksum mismatch: {part.name}")
    chunks.append(text)

source = base64.b64decode("".join(chunks), validate=True)
exec(compile(source.decode("utf-8"), "phase4b_bootstrap_compiled.py", "exec"))
