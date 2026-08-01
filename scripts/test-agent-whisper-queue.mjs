import assert from "node:assert/strict";
import {
  WHISPER_QUEUE_LIMITS,
  WhisperSegmentQueue,
} from "../src/homeserver-whisper-queue.js";

function segment(id, bytes) {
  return {
    segment_id: id,
    blob: new Blob([new Uint8Array(bytes)]),
  };
}

{
  const queue = new WhisperSegmentQueue({ maxSegments: 3, maxBytes: 12 });
  assert.deepEqual(queue.enqueue(segment("one", 3)), {
    accepted: true,
    length: 1,
    byteLength: 3,
  });
  assert.deepEqual(queue.enqueue(segment("two", 4)), {
    accepted: true,
    length: 2,
    byteLength: 7,
  });
  assert.equal(queue.shift()?.segment_id, "one");
  assert.equal(queue.length, 1);
  assert.equal(queue.byteLength, 4);
  assert.equal(queue.shift()?.segment_id, "two");
  assert.equal(queue.length, 0);
  assert.equal(queue.byteLength, 0);
}

{
  const queue = new WhisperSegmentQueue({ maxSegments: 2, maxBytes: 8 });
  assert.equal(queue.enqueue(segment("duplicate", 2)).accepted, true);
  assert.deepEqual(queue.enqueue(segment("duplicate", 2)), {
    accepted: false,
    reason: "duplicate",
  });
  assert.equal(queue.enqueue(segment("second", 4)).accepted, true);
  assert.deepEqual(queue.enqueue(segment("third", 1)), {
    accepted: false,
    reason: "capacity",
  });
}

{
  const queue = new WhisperSegmentQueue({ maxSegments: 3, maxBytes: 5 });
  assert.deepEqual(queue.enqueue(segment("oversize", 6)), {
    accepted: false,
    reason: "capacity",
  });
  assert.deepEqual(queue.enqueue({ segment_id: "missing-blob" }), {
    accepted: false,
    reason: "invalid",
  });
  assert.equal(queue.enqueue(segment("kept", 5)).accepted, true);
  queue.clear();
  assert.equal(queue.length, 0);
  assert.equal(queue.byteLength, 0);
}

assert.equal(WHISPER_QUEUE_LIMITS.maxSegments, 6);
assert.equal(WHISPER_QUEUE_LIMITS.maxBytes, 64 * 1024 * 1024);
console.log("Phase 23C bounded Whisper queue tests passed.");
