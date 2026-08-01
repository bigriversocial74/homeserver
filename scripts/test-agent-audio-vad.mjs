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
  feed(engine, 1_230, 1_650, -24, events);
  feed(engine, 1_680, 2_700, -65, events);
  assert.equal(events.filter((event) => event.event === "speech_start").length, 1);
  assert.equal(events.filter((event) => event.event === "speech_end").length, 1);
  const start = events.find((event) => event.event === "speech_start");
  const end = events.find((event) => event.event === "speech_end");
  assert.ok(start.speechMs >= engine.options.attackMs);
  assert.ok(end.silenceMs >= engine.options.silenceHangoverMs);
}

{
  const engine = new AdaptiveVadEngine({
    calibrationMs: 300,
    attackMs: 60,
    maxSegmentMs: 2_000,
  });
  feed(engine, 0, 360, -62);
  const events = [];
  feed(engine, 390, 2_700, -20, events);
  assert.equal(events.filter((event) => event.event === "speech_start").length, 1);
  assert.ok(events.some((event) => event.event === "segment_limit"));
}

{
  const engine = new AdaptiveVadEngine();
  feed(engine, 0, 1_500, -48);
  const firstFloor = engine.noiseFloorDb;
  feed(engine, 1_530, 3_000, -42);
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
  "Phase 23B adaptive VAD tests passed: calibration, attack rejection, hysteresis, "
    + "silence hangover, segment limits, adaptive noise floor, and input boundaries.",
);
