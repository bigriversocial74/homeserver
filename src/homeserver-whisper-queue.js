const DEFAULT_MAX_SEGMENTS = 6;
const DEFAULT_MAX_BYTES = 64 * 1024 * 1024;

export class WhisperSegmentQueue {
  constructor({
    maxSegments = DEFAULT_MAX_SEGMENTS,
    maxBytes = DEFAULT_MAX_BYTES,
  } = {}) {
    if (!Number.isSafeInteger(maxSegments) || maxSegments < 1) {
      throw new TypeError("Whisper queue maxSegments must be a positive integer.");
    }
    if (!Number.isSafeInteger(maxBytes) || maxBytes < 1) {
      throw new TypeError("Whisper queue maxBytes must be a positive integer.");
    }
    this.maxSegments = maxSegments;
    this.maxBytes = maxBytes;
    this.items = [];
    this.bytes = 0;
    this.segmentIds = new Set();
  }

  get length() {
    return this.items.length;
  }

  get byteLength() {
    return this.bytes;
  }

  enqueue(detail) {
    const segmentId = String(detail?.segment_id || "").trim();
    const blob = detail?.blob;
    if (!segmentId || !(blob instanceof Blob) || blob.size < 1) {
      return { accepted: false, reason: "invalid" };
    }
    if (this.segmentIds.has(segmentId)) {
      return { accepted: false, reason: "duplicate" };
    }
    if (
      blob.size > this.maxBytes
      || this.items.length >= this.maxSegments
      || this.bytes + blob.size > this.maxBytes
    ) {
      return { accepted: false, reason: "capacity" };
    }
    this.items.push({ ...detail, segment_id: segmentId, blob });
    this.segmentIds.add(segmentId);
    this.bytes += blob.size;
    return {
      accepted: true,
      length: this.items.length,
      byteLength: this.bytes,
    };
  }

  shift() {
    const item = this.items.shift() || null;
    if (!item) return null;
    this.segmentIds.delete(item.segment_id);
    this.bytes = Math.max(0, this.bytes - item.blob.size);
    return item;
  }

  clear() {
    this.items.length = 0;
    this.segmentIds.clear();
    this.bytes = 0;
  }
}

export const WHISPER_QUEUE_LIMITS = Object.freeze({
  maxSegments: DEFAULT_MAX_SEGMENTS,
  maxBytes: DEFAULT_MAX_BYTES,
});
