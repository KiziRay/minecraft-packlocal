import { SFX_MUTED_STORAGE_KEY, SFX_VOLUME_STORAGE_KEY } from "../core/storage.js";

export function createSfxControls() {
  let sfxVolume = 0.55;
  let sfxMuted = false;
  let sfxAudioCtx = null;
  let sfxLastPlayAt = 0;
  let sfxLastErrorAt = 0;
  let sfxLastSuccessAt = 0;

const SFX_THROTTLE_MS = 90;
const SFX_ERROR_GUARD_MS = 1500;
const SFX_SUCCESS_GUARD_MS = 2500;
// 優先用很小的本地 ogg 檔；success/error 仍以 WebAudio tone 當保底（避免非使用者觸發時瀏覽器拒絕自動播放）。
const SFX_AUDIO_URLS = {
  click: "assets/audio/pickup2.ogg",
  toggle: "assets/audio/chip-lay-1.ogg",
  tick: "assets/audio/chips-stack-2.ogg",
  scroll: "assets/audio/chips-stack-5.ogg",
};
const sfxAudioPool = {};

function clampSfxVolume(v) {
  const n = Number(v);
  if (!Number.isFinite(n)) return 0.55;
  return Math.min(1, Math.max(0, n));
}

function ensureSfxAudioContext() {
  if (sfxAudioCtx) return sfxAudioCtx;
  const AudioCtx = window.AudioContext || window.webkitAudioContext;
  if (!AudioCtx) return null;
  try {
    sfxAudioCtx = new AudioCtx();
    return sfxAudioCtx;
  } catch (_) {
    sfxAudioCtx = null;
    return null;
  }
}

function playSfx(kind) {
  if (sfxMuted) return;
  const vol = clampSfxVolume(sfxVolume);
  if (!vol) return;
  const now = Date.now();
  if (now - sfxLastPlayAt < SFX_THROTTLE_MS) return;
  sfxLastPlayAt = now;

  const audioUrl = SFX_AUDIO_URLS[kind];
  if (audioUrl) {
    try {
      const a = sfxAudioPool[kind] || new Audio(audioUrl);
      sfxAudioPool[kind] = a;
      a.volume = vol;
      a.currentTime = 0;
      const p = a.play();
      if (p && typeof p.catch === "function") p.catch(() => {});
      return; // 由於 click/toggle/tick/scroll 都是使用者觸發，預期可以播放成功
    } catch (_) {
      /* fallback: 改用 WebAudio tone */
    }
  }

  const ctx = ensureSfxAudioContext();
  if (!ctx) return;
  try {
    if (ctx.state === "suspended" && typeof ctx.resume === "function") ctx.resume().catch(() => null);
  } catch (_) {
    /* ignore */
  }

  const t0 = ctx.currentTime || 0;
  const makeTone = (freq, durMs, wave = "sine", gainMul = 1) => {
    const osc = ctx.createOscillator();
    const gain = ctx.createGain();
    osc.type = wave;
    osc.frequency.setValueAtTime(freq, t0);
    const peak = Math.max(0.001, vol * 0.22 * gainMul);
    gain.gain.setValueAtTime(0.0001, t0);
    gain.gain.exponentialRampToValueAtTime(peak, t0 + 0.01);
    gain.gain.exponentialRampToValueAtTime(0.0001, t0 + durMs / 1000);
    osc.connect(gain);
    gain.connect(ctx.destination);
    osc.start(t0);
    osc.stop(t0 + durMs / 1000 + 0.02);
  };

  // 目標：避免「吵」；用短包絡 + 少量頻率變化。
  switch (kind) {
    case "success":
      makeTone(1046.5, 95, "triangle", 1.0);
      makeTone(1318.5, 95, "sine", 0.8);
      break;
    case "error":
      makeTone(220, 115, "sawtooth", 0.9);
      makeTone(165, 115, "square", 0.35);
      break;
    case "tick":
      makeTone(740, 45, "sine", 0.7);
      break;
    case "scroll":
      makeTone(520, 55, "sine", 0.45);
      break;
    case "toggle":
      makeTone(660, 70, "sine", 0.6);
      break;
    case "click":
    default:
      makeTone(880, 55, "sine", 0.55);
      break;
  }
}

function maybePlaySfxError() {
  const now = Date.now();
  if (now - sfxLastErrorAt < SFX_ERROR_GUARD_MS) return;
  sfxLastErrorAt = now;
  playSfx("error");
}

function maybePlaySfxSuccess(message) {
  const now = Date.now();
  if (now - sfxLastSuccessAt < SFX_SUCCESS_GUARD_MS) return;
  const m = String(message || "");
  if (!/全部完成|補翻完成|補譯完成|字體包完成|修復完成|已套用|完成/.test(m)) return;
  sfxLastSuccessAt = now;
  playSfx("success");
}

/**
 * 音效偏好。控制項在獨立設定視窗（settings.html），主視窗只讀 localStorage
 * 的初值，之後由設定視窗送來的 mcpl:settings-updated 即時更新。
 */
function applySfxPrefs({ muted, volume } = {}) {
  if (muted !== undefined && muted !== null) sfxMuted = muted === true || muted === "1";
  if (volume !== undefined && volume !== null) sfxVolume = clampSfxVolume(volume);
  return { muted: sfxMuted, volume: sfxVolume };
}

function initSfxControls() {
  try {
    const storedVol = localStorage.getItem(SFX_VOLUME_STORAGE_KEY);
    if (storedVol !== null) sfxVolume = clampSfxVolume(storedVol);
  } catch (_) {
    /* ignore */
  }
  try {
    const storedMuted = localStorage.getItem(SFX_MUTED_STORAGE_KEY);
    sfxMuted = storedMuted === "1";
  } catch (_) {
    /* ignore */
  }

  // 使用者點擊：用事件委派觸發 click，避免「滾動/拖曳」造成大量雜訊。
  document.addEventListener(
    "click",
    (ev) => {
      const t = ev.target;
      if (t && t.tagName === "INPUT" && t.type === "checkbox") {
        const id = String(t.id || "");
        if (id === "sfx-muted") return;
        if (sfxMuted) return;
        playSfx("toggle");
        return;
      }
      const el = t && t.closest ? t.closest("button, a") : null;
      if (!el) return;
      if (el instanceof HTMLInputElement) return;
      if (el.tagName === "A" && !el.getAttribute("href")) return;
      const id = String(el.id || "");
      if (!id) return;
      if (!/^btn-|^winbar-|^btn-win-/.test(id) && !el.classList.contains("wb-btn")) {
        return;
      }
      if (el.disabled) return;
      if (sfxMuted) return;
      playSfx("click");
    },
    { capture: true, passive: true }
  );
}


  return { initSfxControls, applySfxPrefs, maybePlaySfxError, maybePlaySfxSuccess };
}
