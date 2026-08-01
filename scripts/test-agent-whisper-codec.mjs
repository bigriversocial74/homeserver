import assert from "node:assert/strict";
import {
  bytesToBase64,
  floatToPcm16,
  WHISPER_MAX_SAMPLES,
  WHISPER_MIN_SAMPLES,
  WHISPER_SAMPLE_RATE,
} from "../src/homeserver-whisper-codec.js";

{
  const samples = new Float32Array(WHISPER_MIN_SAMPLES);
  samples[0] = -1;
  samples[1] = -0.5;
  samples[2] = 0;
  samples[3] = 0.5;
  samples[4] = 1;
  const bytes = floatToPcm16(samples);
  const view = new DataView(bytes.buffer);
  assert.equal(view.getInt16(0, true), -32768);
  assert.equal(view.getInt16(2, true), -16384);
  assert.equal(view.getInt16(4, true), 0);
  assert.equal(view.getInt16(6, true), 16384);
  assert.equal(view.getInt16(8, true), 32767);
  assert.equal(bytes.length, WHISPER_MIN_SAMPLES * 2);
}

{
  const samples = new Float32Array(WHISPER_MIN_SAMPLES);
  samples[0] = Number.NaN;
  samples[1] = Number.POSITIVE_INFINITY;
  const bytes = floatToPcm16(samples);
  const view = new DataView(bytes.buffer);
  assert.equal(view.getInt16(0, true), 0);
  assert.equal(view.getInt16(2, true), 0);
}

{
  assert.throws(() => floatToPcm16(new Float32Array(100)), /boundary/);
  assert.throws(
    () => floatToPcm16(new Float32Array(WHISPER_MAX_SAMPLES + 1)),
    /boundary/,
  );
  assert.throws(() => floatToPcm16([0, 1]), /Float32Array/);
}

{
  const bytes = new Uint8Array([0, 1, 2, 250, 251, 252]);
  assert.equal(bytesToBase64(bytes), Buffer.from(bytes).toString("base64"));
  assert.throws(() => bytesToBase64([1, 2]), /Uint8Array/);
}

assert.equal(WHISPER_SAMPLE_RATE, 16_000);
assert.equal(WHISPER_MAX_SAMPLES / WHISPER_SAMPLE_RATE, 32);

console.log(
  "Phase 23C Whisper PCM codec tests passed: clipping, signed conversion, "
    + "non-finite zeroing, sample boundaries, and chunk-safe base64 encoding.",
);
