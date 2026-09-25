function parseMcVersionParts(version) {
  const cleaned = String(version || "")
    .trim()
    .match(/^[\d.]+/);
  if (!cleaned) return null;
  const parts = cleaned[0]
    .split(".")
    .filter(Boolean)
    .map((p) => Number(p))
    .filter((n) => Number.isFinite(n));
  return parts.length >= 2 ? parts : null;
}

export function isSupportedMinecraftVersion(version) {
  const parts = parseMcVersionParts(version);
  if (!parts) return false;
  if (parts[0] >= 26) return true;
  if (parts[0] !== 1) return false;
  if (parts[1] > 13) return true;
  if (parts[1] < 13) return false;
  return true; // 1.13 / 1.13.x
}

export function unsupportedVersionMessage(version) {
  return (
    "本工具僅支援 Minecraft 1.13 以上（含年份版 26.x），偵測到 " +
    version +
    "，無法翻譯。"
  );
}
