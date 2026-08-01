export const VAD_DEFAULTS = Object.freeze({
  algorithmVersion: "homeserver-adaptive-rms-v1",
  frameMs: 30,
  calibrationMs: 900,
  attackMs: 90,
  silenceHangoverMs: 720,
  minSpeechMs: 240,
  maxSegmentMs: 30_000,
  noiseAdaptation: 0.055,
  initialNoiseFloorDb: -72,
  minNoiseFloorDb: -90,
  maxNoiseFloorDb: -24,
  startMarginDb: 12,
  stopMarginDb: 7,
  minStartThresholdDb: -52,
  maxStartThresholdDb: -28,
  minStopThresholdDb: -60,
  maxStopThresholdDb: -35,
});

function clamp(value, minimum, maximum) {
  return Math.min(maximum, Math.max(minimum, value));
}

function finiteNumber(value, label) {
  const number = Number(value);
  if (!Number.isFinite(number)) throw new TypeError(`${label} must be finite`);
  return number;
}

function boundedOption(options, key, minimum, maximum) {
  const value = finiteNumber(options[key], key);
  if (value < minimum || value > maximum) {
    throw new RangeError(`${key} is outside its supported boundary`);
  }
  return value;
}

export function rmsToDb(rms) {
  const value = finiteNumber(rms, "rms");
  if (value <= 0) return -120;
  return clamp(20 * Math.log10(value), -120, 0);
}

export class AdaptiveVadEngine {
  constructor(overrides = {}) {
    const options = { ...VAD_DEFAULTS, ...overrides };
    this.options = Object.freeze({
      algorithmVersion: String(options.algorithmVersion || VAD_DEFAULTS.algorithmVersion),
      frameMs: boundedOption(options, "frameMs", 10, 100),
      calibrationMs: boundedOption(options, "calibrationMs", 300, 5_000),
      attackMs: boundedOption(options, "attackMs", 30, 500),
      silenceHangoverMs: boundedOption(options, "silenceHangoverMs", 200, 3_000),
      minSpeechMs: boundedOption(options, "minSpeechMs", 100, 2_000),
      maxSegmentMs: boundedOption(options, "maxSegmentMs", 2_000, 120_000),
      noiseAdaptation: boundedOption(options, "noiseAdaptation", 0.001, 0.25),
      initialNoiseFloorDb: boundedOption(options, "initialNoiseFloorDb", -100, -20),
      minNoiseFloorDb: boundedOption(options, "minNoiseFloorDb", -120, -30),
      maxNoiseFloorDb: boundedOption(options, "maxNoiseFloorDb", -60, -10),
      startMarginDb: boundedOption(options, "startMarginDb", 4, 30),
      stopMarginDb: boundedOption(options, "stopMarginDb", 2, 20),
      minStartThresholdDb: boundedOption(options, "minStartThresholdDb", -80, -20),
      maxStartThresholdDb: boundedOption(options, "maxStartThresholdDb", -50, -5),
      minStopThresholdDb: boundedOption(options, "minStopThresholdDb", -90, -25),
      maxStopThresholdDb: boundedOption(options, "maxStopThresholdDb", -60, -10),
    });
    if (this.options.stopMarginDb >= this.options.startMarginDb) {
      throw new RangeError("stopMarginDb must remain below startMarginDb");
    }
    if (this.options.minStopThresholdDb > this.options.minStartThresholdDb) {
      throw new RangeError("stop threshold floor must not exceed the start threshold floor");
    }
    this.reset(0, true);
  }

  reset(nowMs = 0, resetNoise = false) {
    const now = finiteNumber(nowMs, "nowMs");
    if (resetNoise || !Number.isFinite(this.noiseFloorDb)) {
      this.noiseFloorDb = this.options.initialNoiseFloorDb;
    }
    this.startedAtMs = now;
    this.lastFrameAtMs = now;
    this.candidateSpeechAtMs = null;
    this.silenceAtMs = null;
    this.speechStartedAtMs = null;
    this.speaking = false;
    this.lastDb = -120;
    this.frameCount = 0;
    return this.snapshot(now);
  }

  resetSpeech(nowMs = this.lastFrameAtMs) {
    const now = finiteNumber(nowMs, "nowMs");
    this.lastFrameAtMs = now;
    this.candidateSpeechAtMs = null;
    this.silenceAtMs = null;
    this.speechStartedAtMs = null;
    this.speaking = false;
    return this.snapshot(now);
  }

  thresholds() {
    const startThresholdDb = clamp(
      this.noiseFloorDb + this.options.startMarginDb,
      this.options.minStartThresholdDb,
      this.options.maxStartThresholdDb,
    );
    const stopThresholdDb = Math.min(
      startThresholdDb - 1,
      clamp(
        this.noiseFloorDb + this.options.stopMarginDb,
        this.options.minStopThresholdDb,
        this.options.maxStopThresholdDb,
      ),
    );
    return { startThresholdDb, stopThresholdDb };
  }

  snapshot(nowMs = this.lastFrameAtMs, event = null) {
    const now = finiteNumber(nowMs, "nowMs");
    const { startThresholdDb, stopThresholdDb } = this.thresholds();
    return {
      event,
      speaking: this.speaking,
      calibrated: now - this.startedAtMs >= this.options.calibrationMs,
      algorithmVersion: this.options.algorithmVersion,
      rmsDb: this.lastDb,
      noiseFloorDb: this.noiseFloorDb,
      startThresholdDb,
      stopThresholdDb,
      speechMs: this.speechStartedAtMs === null ? 0 : Math.max(0, now - this.speechStartedAtMs),
      silenceMs: this.silenceAtMs === null ? 0 : Math.max(0, now - this.silenceAtMs),
      frameCount: this.frameCount,
    };
  }

  update(rmsDb, nowMs) {
    const levelDb = clamp(finiteNumber(rmsDb, "rmsDb"), -120, 0);
    const now = finiteNumber(nowMs, "nowMs");
    if (now < this.lastFrameAtMs) throw new RangeError("VAD timestamps must be monotonic");

    this.lastFrameAtMs = now;
    this.lastDb = levelDb;
    this.frameCount += 1;

    const calibrated = now - this.startedAtMs >= this.options.calibrationMs;
    const thresholdsBeforeAdaptation = this.thresholds();
    const quietEnoughToLearn = !this.speaking
      && levelDb < thresholdsBeforeAdaptation.startThresholdDb;
    if (quietEnoughToLearn) {
      const alpha = this.options.noiseAdaptation;
      this.noiseFloorDb = clamp(
        this.noiseFloorDb * (1 - alpha) + levelDb * alpha,
        this.options.minNoiseFloorDb,
        this.options.maxNoiseFloorDb,
      );
    }

    const { startThresholdDb, stopThresholdDb } = this.thresholds();

    if (!this.speaking) {
      this.silenceAtMs = null;
      if (!calibrated || levelDb < startThresholdDb) {
        this.candidateSpeechAtMs = null;
        return this.snapshot(now);
      }
      if (this.candidateSpeechAtMs === null) this.candidateSpeechAtMs = now;
      const requiredSpeechMs = Math.max(
        this.options.attackMs,
        this.options.minSpeechMs,
      );
      if (now - this.candidateSpeechAtMs >= requiredSpeechMs) {
        this.speaking = true;
        this.speechStartedAtMs = this.candidateSpeechAtMs;
        this.candidateSpeechAtMs = null;
        return this.snapshot(now, "speech_start");
      }
      return this.snapshot(now);
    }

    const speechMs = Math.max(0, now - this.speechStartedAtMs);
    if (speechMs >= this.options.maxSegmentMs) {
      this.silenceAtMs = null;
      return this.snapshot(now, "segment_limit");
    }

    if (levelDb >= stopThresholdDb) {
      this.silenceAtMs = null;
      return this.snapshot(now);
    }
    if (this.silenceAtMs === null) this.silenceAtMs = now;
    if (
      speechMs >= this.options.minSpeechMs
      && now - this.silenceAtMs >= this.options.silenceHangoverMs
    ) {
      return this.snapshot(now, "speech_end");
    }
    return this.snapshot(now);
  }
}
