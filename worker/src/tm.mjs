// 共享 TM／glossary 多數決：取消永久 conflict 凍結。
// 舊字串紀錄＝1 票；舊 packs 鍵數可當票數；跨包需 ≥2，同包或 pack.* 層 ≥1。

export const TM_MAX_ZH_LEN = 8192;
export const GLOSSARY_MAX_ZH_LEN = 400;
export const TM_CROSS_PACK_MIN_VOTES = 2;
export const TM_PACK_MIN_VOTES = 1;
export const TM_PACKS_CAP = 16;
/** 一票最多記幾個 mod 版本（防止記錄無限膨脹） */
export const TM_MODS_CAP = 8;
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

/**
 * 這一票是在哪些「模組版本」下貢獻的。
 *
 * 值長得像 `create-1.20.1-0.5.1.f`（mod 檔名，含版本）。留這個是為了分辨
 * 「不同整合包，但同一個模組的同一版本」——那種情況兩邊講的是同一個模組的
 * 同一個字串，跟同一個整合包沒有差別，所以一票就可以採用。
 *
 * 舊記錄沒有這個欄位 → 回空物件 → 一切照舊需要兩票，不會讓既有資料失效。
 */
function cloneMods(value) {
  if (!value || typeof value !== "object" || Array.isArray(value)) return {};
  const out = {};
  for (const key of Object.keys(value)) {
    if (typeof key === "string" && key) out[key] = 1;
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
      votes.push({ zh, n, packs: clonePacks(item.packs), mods: cloneMods(item.mods) });
      if (votes.length >= TM_VOTES_CAP) break;
    }
  }
  if (!votes.length && fallbackZh) {
    const packCount = Object.keys(fallbackPacks || {}).length;
    votes.push({
      zh: fallbackZh,
      n: Math.max(1, packCount),
      packs: clonePacks(fallbackPacks),
      mods: {},
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
  return (
    pickWinningVote(record.votes) ||
    (record.zh ? { zh: record.zh, n: 1, packs: record.packs, mods: {} } : null)
  );
}

function normalizeQueryPks(queryPks) {
  if (Array.isArray(queryPks)) return queryPks.filter((pk) => typeof pk === "string" && pk);
  return typeof queryPks === "string" && queryPks ? [queryPks] : [];
}

function samePackHit(record, winner, queryPks) {
  const pks = normalizeQueryPks(queryPks);
  if (!pks.length) return false;
  for (const pk of pks) {
    if (Object.prototype.hasOwnProperty.call(record.packs || {}, pk)) return true;
    if (winner?.packs && Object.prototype.hasOwnProperty.call(winner.packs, pk)) return true;
  }
  return false;
}

/**
 * 這一票有沒有來自「同一個模組的同一個版本」。
 *
 * 整合包之間大量共用同樣的模組。`create:item.wrench` 在任何裝了同一版
 * Create 的包裡都是同一個字串、同一個意思——不該因為整合包不同就要求兩票。
 *
 * 只認**完全相同**的 mod 檔識別（含版本）。模組改版時字串可能整個換掉，
 * 那時就退回一般的跨包門檻。
 */
function sameModHit(winner, queryMod) {
  if (typeof queryMod !== "string" || !queryMod) return false;
  return Boolean(winner?.mods && Object.prototype.hasOwnProperty.call(winner.mods, queryMod));
}

/**
 * 這筆記錄可不可以用。
 *
 * 需要幾票：
 * | 情況 | 票數 | 為什麼 |
 * |---|---|---|
 * | 同一個整合包 | 1 | 就是你自己（或同一包的別人）翻的 |
 * | 同一個模組、同一個版本（不同包） | 1 | **本次新增**：來源檔案逐字相同，等同同一包 |
 * | 整合包覆寫層（`pack.` 前綴） | 1 | 維持原本行為，本次沒有改動 |
 * | 其他（跨包、跨版本） | 2 | 維持現況，防止單一來源的壞翻譯擴散 |
 *
 * `queryMod` 沒帶（舊版工具）或記錄裡沒有 `mods`（舊資料）時，
 * 一律退回原本的規則——既有資料與舊版工具的行為完全不變。
 */
export function tmCanUse(value, ctx, queryPks, ns, queryMod) {
  const record = tmNormalizeRecord(value);
  if (!record) return null;
  if (record.ctx && ctx && record.ctx !== ctx) return null;
  const winner = winningVoteForLookup(record);
  if (!winner || !winner.zh || !String(winner.zh).trim()) return null;
  const packLayer = isPackLayerNs(ns);
  // packLayer 與 samePackHit 的行為維持原樣，只多加一條「同模組同版本」。
  // 這是刻意的：這次要放寬的是跨整合包的模組層重用，不是改動既有的信任規則。
  const oneVoteEnough =
    packLayer || samePackHit(record, winner, queryPks) || sameModHit(winner, queryMod);
  const need = oneVoteEnough ? TM_PACK_MIN_VOTES : TM_CROSS_PACK_MIN_VOTES;
  if (winner.n >= need) return winner.zh;
  return null;
}

export function tmMerge(target, key, next) {
  const incomingZh = typeof next?.zh === "string" ? next.zh.trim() : "";
  if (!incomingZh) return "skip";
  const incomingPacks = clonePacks(next.packs);
  const incomingMods = cloneMods(next.mods);
  const incomingCtx = typeof next.ctx === "string" ? next.ctx : "";
  const previous = tmNormalizeRecord(target[key]);
  if (!previous) {
    const votePacks = clonePacks(incomingPacks);
    target[key] = {
      zh: incomingZh,
      ctx: incomingCtx,
      packs: clonePacks(incomingPacks),
      votes: [{ zh: incomingZh, n: 1, packs: votePacks, mods: cloneMods(incomingMods) }],
    };
    return "accepted";
  }
  let vote = previous.votes.find((item) => item.zh === incomingZh);
  let variant = false;
  if (!vote) {
    variant = true;
    if (previous.votes.length >= TM_VOTES_CAP) {
      return "duplicate";
    }
    vote = { zh: incomingZh, n: 0, packs: {}, mods: {} };
    previous.votes.push(vote);
  }
  // 記下這一票在哪些 mod 版本下出現過。**不影響票數**——票數仍然只由
  // 整合包決定，這裡只是替 tmCanUse 留下「同模組同版本」的判斷依據。
  if (!vote.mods) vote.mods = {};
  for (const mod of Object.keys(incomingMods)) {
    if (Object.keys(vote.mods).length >= TM_MODS_CAP) break;
    vote.mods[mod] = 1;
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
