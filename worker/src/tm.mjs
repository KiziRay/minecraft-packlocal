// 共享 TM／glossary 多數決：取消永久 conflict 凍結。
// 舊字串紀錄＝1 票；舊 packs 鍵數可當票數；跨包需 ≥2，同包或 pack.* 層 ≥1。

export const TM_MAX_ZH_LEN = 8192;
export const GLOSSARY_MAX_ZH_LEN = 400;
export const TM_CROSS_PACK_MIN_VOTES = 2;
export const TM_PACK_MIN_VOTES = 1;
export const TM_PACKS_CAP = 16;
export const TM_VOTES_CAP = 8;

export function utf8ByteLength(value) {
  return new TextEncoder().encode(String(value || "")).length;
}

export function tmZhAcceptable(zh, maxLen = TM_MAX_ZH_LEN) {
  const trimmed = typeof zh === "string" ? zh.trim() : "";
  return !!trimmed && utf8ByteLength(trimmed) <= maxLen;
}

export function isPackLayerNs(ns) {
  return typeof ns === "string" && ns.startsWith("pack.");
}

function clonePacks(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const out = {};
  for (const [key, name] of Object.entries(value)) {
    if (typeof key === "string" && key) out[key] = typeof name === "string" ? name : "";
  }
  return out;
}

function normalizeVoteList(rawVotes, fallbackZh, fallbackPacks) {
  const votes = [];
  if (Array.isArray(rawVotes)) {
    for (const item of rawVotes) {
      if (!item || typeof item !== "object") continue;
      const zh = typeof item.zh === "string" ? item.zh.trim() : "";
      if (!zh) continue;
      const n = Math.max(1, Math.floor(Number(item.n) || 1));
      votes.push({ zh, n, packs: clonePacks(item.packs) });
      if (votes.length >= TM_VOTES_CAP) break;
    }
  }
  if (!votes.length && fallbackZh) {
    const packCount = Object.keys(fallbackPacks || {}).length;
    votes.push({
      zh: fallbackZh,
      n: Math.max(1, packCount),
      packs: clonePacks(fallbackPacks),
    });
  }
  return votes;
}

export function pickWinningVote(votes) {
  if (!Array.isArray(votes) || !votes.length) return null;
  let best = votes[0];
  for (const vote of votes.slice(1)) {
    if (vote.n > best.n) best = vote;
  }
  const tied = votes.filter((vote) => vote.n === best.n);
  if (tied.length > 1) return null;
  return best;
}

export function tmNormalizeRecord(value) {
  if (typeof value === "string") {
    const zh = value.trim();
    if (!zh) return null;
    return {
      zh,
      ctx: "",
      packs: {},
      votes: [{ zh, n: 1, packs: {} }],
    };
  }
  if (!value || typeof value !== "object") return null;
  const packs = clonePacks(value.packs);
  const zh = typeof value.zh === "string" ? value.zh.trim() : "";
  const votes = normalizeVoteList(value.votes, zh, packs);
  if (!votes.length) return null;
  const winner = pickWinningVote(votes);
  return {
    zh: winner ? winner.zh : zh || votes[0].zh,
    ctx: typeof value.ctx === "string" ? value.ctx : "",
    packs,
    votes,
  };
}

function winningVoteForLookup(record) {
  return pickWinningVote(record.votes) || (record.zh ? { zh: record.zh, n: 1, packs: record.packs } : null);
}

function samePackHit(record, winner, queryPk) {
  if (!queryPk) return false;
  if (Object.prototype.hasOwnProperty.call(record.packs || {}, queryPk)) return true;
  return !!(winner?.packs && Object.prototype.hasOwnProperty.call(winner.packs, queryPk));
}

export function tmCanUse(value, ctx, queryPk, ns) {
  const record = tmNormalizeRecord(value);
  if (!record) return null;
  if (record.ctx && ctx && record.ctx !== ctx) return null;
  const winner = winningVoteForLookup(record);
  if (!winner || !winner.zh || !String(winner.zh).trim()) return null;
  const packLayer = isPackLayerNs(ns);
  const need =
    packLayer || samePackHit(record, winner, queryPk) ? TM_PACK_MIN_VOTES : TM_CROSS_PACK_MIN_VOTES;
  if (winner.n >= need) return winner.zh;
  return null;
}

export function tmMerge(target, key, next) {
  const incomingZh = typeof next?.zh === "string" ? next.zh.trim() : "";
  if (!incomingZh) return "skip";
  const incomingPacks = clonePacks(next.packs);
  const incomingCtx = typeof next.ctx === "string" ? next.ctx : "";
  const previous = tmNormalizeRecord(target[key]);
  if (!previous) {
    const votePacks = clonePacks(incomingPacks);
    target[key] = {
      zh: incomingZh,
      ctx: incomingCtx,
      packs: clonePacks(incomingPacks),
      votes: [{ zh: incomingZh, n: 1, packs: votePacks }],
    };
    return "accepted";
  }
  let vote = previous.votes.find((item) => item.zh === incomingZh);
  let variant = false;
  if (!vote) {
    variant = true;
    if (previous.votes.length >= TM_VOTES_CAP) {
      target[key] = previous;
      return "accepted";
    }
    vote = { zh: incomingZh, n: 0, packs: {} };
    previous.votes.push(vote);
  }
  const pk = Object.keys(incomingPacks)[0];
  if (pk) {
    if (!vote.packs[pk]) {
      vote.n += 1;
      vote.packs[pk] = incomingPacks[pk];
    }
    if (!previous.packs[pk] && Object.keys(previous.packs).length < TM_PACKS_CAP) {
      previous.packs[pk] = incomingPacks[pk];
    }
  } else if (vote.n < 1) {
    vote.n = 1;
  }
  if (!previous.ctx && incomingCtx) previous.ctx = incomingCtx;
  const winner = pickWinningVote(previous.votes);
  if (winner) previous.zh = winner.zh;
  target[key] = previous;
  if (variant) return "variant";
  if (pk && vote.packs[pk] && vote.n >= 1) {
    return vote.n === 1 && Object.keys(vote.packs).length === 1 ? "accepted" : "accepted";
  }
  return "duplicate";
}
