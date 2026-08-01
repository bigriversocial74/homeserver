import assert from "node:assert/strict";
import { AdaptiveVadEngine, rmsToDb } from "../src/homeserver-vad-engine.js";

function feed(engine, startMs, endMs, db, events = []) {
  for (let now = startMs; now <= endMs; now += engine.options.frameMs) {
    const snapshot = engine.update(db, now);
    if (snapshot.event) events.push({ now, ...snapshot });
  }
  return events;
}

assert.equal(rmsToDb(0), -120);
assert.ok(Math.abs(rmsToDb(0.1) + 20) < 0.001);

{
  const engine = new AdaptiveVadEngine();
  const events = feed(engine, 0, 1_200, -58);
  assert.equal(events.length, 0, "room calibration must not create speech");
  const snapshot = engine.snapshot(1_200);
  assert.equal(snapshot.calibrated, true);
  assert.ok(snapshot.noiseFloorDb < -55 && snapshot.noiseFloorDb > -75);
  assert.ok(snapshot.startThresholdDb > snapshot.stopThresholdDb);
}

{
  const engine = new AdaptiveVadEngine();
  feed(engine, 0, 1_200, -60);
  const events = [];
  feed(engine, 1_230, 1_290, -24, events);
  feed(engine, 1_320, 1_650, -60, events);
  assert.equal(
    events.some((event) => event.event === "speech_start"),
    false,
    "a sub-attack transient must be rejected",
  );
}

{
  const engine = new AdaptiveVadEngine();
  feed(engine, 0, 1_200, -60);
  const events = [];
  feed(engine, 1_230, 1_380, -24, events);
  feed(engine, 1_410, 2_400, -65, events);
  assert.equal(
    events.some((event) => event.event === "speech_start"),
    false,
    "speech shorter than the minimum sustained boundary must be rejected",
  );
}

{
  const engine = new AdaptiveVadEngine();
  feed(engine, 0, 1_200, -60);
  const events = [];
  feed(engine, 1_230, 1_650, -24, events);
  feed(engine, 1_680, 2_700, -65, events);
  assert.equal(events.filter((event) => event.event === "speech_start").length, 1);
  assert.equal(events.filter((event) => event.event === "speech_end").length, 1);
  const start = events.find((event) => event.event === "speech_start");
  const end = events.find((event) => event.event === "speech_end");
  assert.ok(start.speechMs >= engine.options.minSpeechMs);
  assert.ok(end.silenceMs >= engine.options.silenceHangoverMs);
  const afterBoundary = engine.snapshot(2_700);
  assert.equal(afterBoundary.speaking, false);
  assert.equal(afterBoundary.speechMs, 0);
}

{
  const engine = new AdaptiveVadEngine({
    calibrationMs: 300,
    attackMs: 60,
    maxSegmentMs: 2_000,
  });
  feed(engine, 0, 360, -62);
  const events = [];
  feed(engine, 390, 3_000, -20, events);
  assert.equal(events.filter((event) => event.event === "speech_start").length, 1);
  assert.equal(
    events.filter((event) => event.event === "segment_limit").length,
    1,
    "the maximum-segment boundary must be edge-triggered",
  );
  engine.resetSpeech(3_030);
  assert.equal(engine.snapshot(3_030).speaking, false);
}

{
  const engine = new AdaptiveVadEngine();
  const firstEvents = feed(engine, 0, 1_500, -64);
  const firstFloor = engine.noiseFloorDb;
  const secondEvents = feed(engine, 1_530, 3_000, -58);
  assert.equal(firstEvents.length, 0, "ambient calibration must remain non-speech");
  assert.equal(secondEvents.length, 0, "sub-threshold ambient drift must remain non-speech");
  assert.ok(engine.noiseFloorDb > firstFloor, "noise floor must adapt upward locally");
  assert.ok(engine.noiseFloorDb <= engine.options.maxNoiseFloorDb);
}

{
  const engine = new AdaptiveVadEngine();
  assert.throws(() => engine.update(-30, -1), /monotonic/);
  assert.throws(
    () => new AdaptiveVadEngine({ stopMarginDb: 13, startMarginDb: 12 }),
    /below startMarginDb/,
  );
}

console.log(
  "Phase 23B adaptive VAD tests passed: calibration, sustained attack, hysteresis, "
    + "one-shot silence and segment boundaries, adaptive noise floor, and input limits.",
);
