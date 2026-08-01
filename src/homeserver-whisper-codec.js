export const WHISPER_SAMPLE_RATE = 16_000;
export const WHISPER_MAX_SECONDS = 32;
export const WHISPER_MAX_SAMPLES = WHISPER_SAMPLE_RATE * WHISPER_MAX_SECONDS;
export const WHISPER_MIN_SAMPLES = 1_600;

function clamp(value, minimum, maximum) {
  return Math.max(minimum, Math.min(maximum, value));
}

export function floatToPcm16(samples) {
  if (!(samples instanceof Float32Array)) {
    throw new TypeError("Whisper audio samples must be a Float32Array.");
  }
  if (samples.length < WHISPER_MIN_SAMPLES || samples.length > WHISPER_MAX_SAMPLES) {
    throw new RangeError("Whisper audio sample count is outside the governed boundary.");
  }
  const output = new Uint8Array(samples.length * 2);
  const view = new DataView(output.buffer);
  for (let index = 0; index < samples.length; index += 1) {
    const value = clamp(Number.isFinite(samples[index]) ? samples[index] : 0, -1, 1);
    const pcm = value < 0 ? Math.round(value * 32768) : Math.round(value * 32767);
    view.setInt16(index * 2, pcm, true);
  }
  return output;
}

export function bytesToBase64(bytes) {
  if (!(bytes instanceof Uint8Array)) {
    throw new TypeError("Whisper PCM bytes must be a Uint8Array.");
  }
  let binary = "";
  const chunkSize = 32 * 1024;
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    const chunk = bytes.subarray(offset, Math.min(bytes.length, offset + chunkSize));
    binary += String.fromCharCode(...chunk);
  }
  return btoa(binary);
}

export async function audioBlobToWhisperPcm(blob) {
  if (!(blob instanceof Blob) || !blob.size) {
    throw new TypeError("A non-empty local audio blob is required.");
  }
  const decodeContext = new AudioContext({ latencyHint: "interactive" });
  try {
    const decoded = await decodeContext.decodeAudioData(await blob.arrayBuffer());
    const durationSeconds = Math.min(decoded.duration, WHISPER_MAX_SECONDS);
    const frameCount = Math.floor(durationSeconds * WHISPER_SAMPLE_RATE);
    if (frameCount < WHISPER_MIN_SAMPLES || frameCount > WHISPER_MAX_SAMPLES) {
      throw new RangeError("Decoded audio duration is outside the local Whisper boundary.");
    }
    const offline = new OfflineAudioContext(1, frameCount, WHISPER_SAMPLE_RATE);
    const source = offline.createBufferSource();
    source.buffer = decoded;
    source.connect(offline.destination);
    source.start(0, 0, durationSeconds);
    const rendered = await offline.startRendering();
    const channel = rendered.getChannelData(0);
    if (channel.length !== frameCount) {
      throw new Error("Local Whisper resampling produced an unexpected sample count.");
    }
    const bytes = floatToPcm16(new Float32Array(channel));
    return {
      pcm16_base64: bytesToBase64(bytes),
      sample_rate_hz: WHISPER_SAMPLE_RATE,
      channels: 1,
      sample_count: channel.length,
      duration_ms: Math.floor((channel.length * 1000) / WHISPER_SAMPLE_RATE),
    };
  } finally {
    await decodeContext.close().catch(() => null);
  }
}
