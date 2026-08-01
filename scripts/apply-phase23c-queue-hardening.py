from __future__ import annotations

from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]


def replace_exact(path: Path, old: str, new: str, label: str) -> None:
    text = path.read_text(encoding="utf-8")
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"Expected one {label} replacement in {path}, found {count}")
    path.write_text(text.replace(old, new), encoding="utf-8")


controller = ROOT / "src/homeserver-agent-whisper.js"
replace_exact(
    controller,
    'import { audioBlobToWhisperPcm } from "./homeserver-whisper-codec.js";\n',
    'import { audioBlobToWhisperPcm } from "./homeserver-whisper-codec.js";\nimport { WhisperSegmentQueue } from "./homeserver-whisper-queue.js";\n',
    "Whisper queue import",
)
replace_exact(
    controller,
    """  partials: new Map(),
};
""",
    """  partials: new Map(),
  queue: new WhisperSegmentQueue(),
};
""",
    "Whisper queue state",
)
replace_exact(
    controller,
    """async function transcribeLocalSegment(detail) {
  if (!detail?.segment_id || !(detail.blob instanceof Blob) || state.active) return;
  const status = await refreshStatus();
""",
    """function queueSegment(detail) {
  const result = state.queue.enqueue(detail);
  if (result.accepted) {
    notify(
      `Queued ${result.length} local utterance${result.length === 1 ? "" : "s"} for transcription.`,
    );
    return;
  }
  if (result.reason === "capacity") {
    notify(
      "The bounded local transcription queue is full; this utterance was not queued.",
      "warning",
    );
  }
}

function drainQueuedSegment() {
  if (state.active || state.busy) return;
  const next = state.queue.shift();
  if (next) void transcribeLocalSegment(next);
}

async function transcribeLocalSegment(detail) {
  if (!detail?.segment_id || !(detail.blob instanceof Blob)) return;
  if (state.active?.segmentId === detail.segment_id) return;
  if (state.active || state.busy) {
    queueSegment(detail);
    return;
  }
  const status = await refreshStatus();
""",
    "bounded FIFO controller",
)
replace_exact(
    controller,
    """  } finally {
    state.active = null;
    state.busy = false;
    scheduleRender();
    await refreshStatus();
  }
}
""",
    """  } finally {
    state.active = null;
    await refreshStatus();
    state.busy = false;
    scheduleRender();
    drainQueuedSegment();
  }
}
""",
    "queue drain lifecycle",
)
replace_exact(
    controller,
    """  if (!validSha256(expected) || state.importing || state.active) {
""",
    """  if (
    !validSha256(expected)
    || state.importing
    || state.active
    || state.busy
    || state.queue.length
  ) {
""",
    "import queue guard",
)
replace_exact(
    controller,
    """  if (state.active || state.importing) return;
""",
    """  if (state.active || state.busy || state.importing || state.queue.length) return;
""",
    "remove queue guard",
)
replace_exact(
    controller,
    """      <button type="button" data-agent-whisper-import ${!validSha256(state.expectedSha256) || state.importing || active ? "disabled" : ""}>${state.importing ? "Importing…" : ready ? "Replace model" : "Import model"}</button>
      <button type="button" data-agent-whisper-remove ${!ready || state.importing || active ? "disabled" : ""}>Remove model</button>
""",
    """      <button type="button" data-agent-whisper-import ${!validSha256(state.expectedSha256) || state.importing || state.busy || active || state.queue.length ? "disabled" : ""}>${state.importing ? "Importing…" : ready ? "Replace model" : "Import model"}</button>
      <button type="button" data-agent-whisper-remove ${!ready || state.importing || state.busy || active || state.queue.length ? "disabled" : ""}>Remove model</button>
""",
    "queued model action guards",
)
replace_exact(
    controller,
    """    ${active ? `<div class="hs-agent-whisper-progress"><i style="width:${progress}%"></i><span>${progress}%</span></div>` : ""}
    ${state.notice ? `<div class="hs-agent-whisper-notice" data-kind="${escapeHtml(state.notice.kind)}">${escapeHtml(state.notice.message)}</div>` : ""}
""",
    """    ${active ? `<div class="hs-agent-whisper-progress"><i style="width:${progress}%"></i><span>${progress}%</span></div>` : ""}
    ${state.queue.length ? `<small>${state.queue.length} utterance${state.queue.length === 1 ? "" : "s"} queued locally · ${state.queue.byteLength.toLocaleString()} bytes ephemeral</small>` : ""}
    ${state.notice ? `<div class="hs-agent-whisper-notice" data-kind="${escapeHtml(state.notice.kind)}">${escapeHtml(state.notice.message)}</div>` : ""}
""",
    "queued utterance status",
)
replace_exact(
    controller,
    """window.addEventListener("pagehide", () => {
  unlistenProgress?.();
  observer?.disconnect();
});
""",
    """window.addEventListener("pagehide", () => {
  state.queue.clear();
  unlistenProgress?.();
  observer?.disconnect();
});
""",
    "ephemeral queue cleanup",
)

validator = ROOT / "scripts/validate-agent-whisper.py"
replace_exact(
    validator,
    'CODEC = (ROOT / "src/homeserver-whisper-codec.js").read_text(encoding="utf-8")\n',
    'CODEC = (ROOT / "src/homeserver-whisper-codec.js").read_text(encoding="utf-8")\nQUEUE = (ROOT / "src/homeserver-whisper-queue.js").read_text(encoding="utf-8")\n',
    "queue validator source",
)
replace_exact(
    validator,
    """    ("Final local transcript is ready", "editable final transcript UX"),
):
""",
    """    ("Final local transcript is ready", "editable final transcript UX"),
    ("WhisperSegmentQueue", "bounded utterance queue integration"),
    ("queueSegment(detail)", "non-dropping active transcription handoff"),
    ("drainQueuedSegment()", "FIFO queue drain"),
    ("state.queue.clear()", "ephemeral queue cleanup"),
):
""",
    "controller queue requirements",
)
replace_exact(
    validator,
    """require(VAD, "homeserver:vad-segment-finalized", "VAD-to-Whisper local blob handoff")
""",
    """require(VAD, "homeserver:vad-segment-finalized", "VAD-to-Whisper local blob handoff")
for token, label in (
    ("DEFAULT_MAX_SEGMENTS = 6", "bounded queue segment count"),
    ("DEFAULT_MAX_BYTES = 64 * 1024 * 1024", "bounded queue byte count"),
    ("this.segmentIds", "duplicate segment rejection"),
    ("this.bytes + blob.size > this.maxBytes", "aggregate byte boundary"),
    ("this.items.shift()", "FIFO queue order"),
):
    require(QUEUE, token, label)
""",
    "queue implementation requirements",
)
replace_exact(
    validator,
    """    ("node --check src/homeserver-whisper-codec.js", "Whisper codec syntax gate"),
    ("node --check src/homeserver-agent-whisper.js", "Whisper controller syntax gate"),
    ("node scripts/test-agent-whisper-codec.mjs", "Whisper codec behavior gate"),
""",
    """    ("node --check src/homeserver-whisper-codec.js", "Whisper codec syntax gate"),
    ("node --check src/homeserver-whisper-queue.js", "Whisper queue syntax gate"),
    ("node --check src/homeserver-agent-whisper.js", "Whisper controller syntax gate"),
    ("node scripts/test-agent-whisper-codec.mjs", "Whisper codec behavior gate"),
    ("node scripts/test-agent-whisper-queue.mjs", "Whisper queue behavior gate"),
""",
    "queue package gates",
)
replace_exact(
    validator,
    """    (CONTROLLER, "indexedDB", "browser persistence"),
""",
    """    (CONTROLLER, "indexedDB", "browser persistence"),
    (CONTROLLER, "|| state.active) return;", "silent active-transcription segment drop"),
""",
    "silent drop prohibition",
)
replace_exact(
    validator,
    """    ROOT / ".github/workflows/phase23c-operation-hardening.yml",
):
""",
    """    ROOT / ".github/workflows/phase23c-operation-hardening.yml",
    ROOT / "scripts/apply-phase23c-queue-hardening.py",
    ROOT / ".github/workflows/phase23c-queue-hardening.yml",
):
""",
    "queue hardening cleanup denylist",
)

print("Applied Phase 23C bounded FIFO utterance queue hardening.")
