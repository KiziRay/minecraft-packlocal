//! B4：本地模型的上下文大小與等待上限怎麼算（依模型大小、記憶體、同時處理數與批次大小，不寫死）。
//!
//! 兩個會讓本地翻譯「看起來壞掉」的硬限制：
//! 1. llama-server 的 `-c` 是**所有同時處理的請求共用**的——開 3 個 slot、`-c 8192`，
//!    每個請求實際只有約 2730 個 token，裝不下「提示＋一批原文＋輸出上限」就會截斷或直接報錯。
//!    所以上下文要用「每個請求需要多少 × 同時處理幾個」回推，再看記憶體放不放得下。
//! 2. 純 CPU 跑的時候，一批 12 條可能要好幾分鐘。等待上限寫死 60～90 秒，
//!    就會「每批都逾時→重送→又逾時」空轉到放棄。等待上限要依批次大小與實測速度估。
//!
//! 所有估算值都標了「待使用者實測」：它們是保守的起點，實測後再調。

use std::sync::atomic::{AtomicU64, Ordering};

const GB: u64 = 1024 * 1024 * 1024;

/// 本地精簡 system prompt ＋ 最多 40 條固定術語，約佔多少 token（待使用者實測）。
pub const SYSTEM_PROMPT_TOKENS: u32 = 900;
/// 每條待譯原文（含 JSON 包裝）平均約佔多少 token（待使用者實測）。
pub const TOKENS_PER_INPUT_ITEM: u32 = 60;
/// 預留給 chat 樣板、特殊 token 的餘裕。
const TEMPLATE_MARGIN_TOKENS: u32 = 256;
/// 系統與工具本身要留的記憶體（待使用者實測）。
const RAM_HEADROOM_BYTES: u64 = 3 * GB;
/// 速度估計（token／秒）：沒有實測值時用。GPU 有放層數／純 CPU（待使用者實測）。
pub const ASSUMED_TPS_GPU: f64 = 20.0;
pub const ASSUMED_TPS_CPU: f64 = 4.0;
/// 單批等待上限的上下界（秒）。
pub const MIN_TIMEOUT_SECS: u64 = 60;
pub const MAX_TIMEOUT_SECS: u64 = 900;

/// 一個請求最多會用到的 token：提示＋一批原文＋輸出上限。
pub fn per_slot_tokens_needed(max_batch_items: u32, max_output_tokens: u32) -> u32 {
    SYSTEM_PROMPT_TOKENS
        .saturating_add(max_batch_items.saturating_mul(TOKENS_PER_INPUT_ITEM))
        .saturating_add(max_output_tokens)
        .saturating_add(TEMPLATE_MARGIN_TOKENS)
}

/// 每個 token 的 KV 快取約佔多少位元組。以模型檔大小粗估：
/// 8B 級（約 7.4 GB 的 gguf）實際約 144 KB／token（待使用者實測）。
pub fn kv_bytes_per_token(model_bytes: u64) -> u64 {
    (model_bytes / 50_000).clamp(32 * 1024, 512 * 1024)
}

/// 依記憶體分的保底上下文（舊規則：≥32 GB 16384、≥16 GB 12288、其餘 8192）。
pub fn baseline_context_for(ram_bytes: u64) -> u32 {
    let gb = ram_bytes / GB;
    if gb >= 32 {
        16384
    } else if gb >= 16 {
        12288
    } else {
        8192
    }
}

/// 啟動時的上下文決策（連同理由，寫進使用者看得到的日誌）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPlan {
    /// 傳給 `-c` 的值
    pub ctx: u32,
    /// 同時處理幾個請求（`--parallel`）
    pub slots: u32,
    /// 每個請求實際分到多少
    pub per_slot: u32,
    /// 每個請求需要多少
    pub needed_per_slot: u32,
    /// 記憶體最多放得下多少（0＝不限／不知道）
    pub ram_cap: u32,
    /// 記憶體不夠放下需要的量：可能會截斷或變慢
    pub tight: bool,
    /// 因記憶體不夠而把同時處理數從幾降下來（0＝沒降）
    pub reduced_from: u32,
}

fn round_up_1k(v: u64) -> u64 {
    v.div_ceil(1024) * 1024
}

/// 算上下文：先求「需要多少」（每個請求需要量 × 同時處理數），不少於依記憶體的保底值；
/// 再用「記憶體扣掉模型與保留量後，放得下多少 KV」封頂——但不會為了封頂而低於需要量
/// 以外的保底（那種情況標成 `tight`，照樣啟動並在日誌說明）。
pub fn plan_context(
    ram_bytes: u64,
    model_bytes: u64,
    slots: u32,
    max_batch_items: u32,
    max_output_tokens: u32,
) -> ContextPlan {
    let plan = plan_context_with_slots(ram_bytes, model_bytes, slots, max_batch_items, max_output_tokens);
    // 審查 1c：記憶體放不下「每個請求需要量 × 同時處理數」時，先降成一次處理一個請求——
    // 每個請求分到全部上下文，比多個請求各分一點（每個都被截斷）穩。
    if plan.tight && plan.slots > 1 {
        let mut single = plan_context_with_slots(ram_bytes, model_bytes, 1, max_batch_items, max_output_tokens);
        single.reduced_from = plan.slots;
        return single;
    }
    plan
}

fn plan_context_with_slots(
    ram_bytes: u64,
    model_bytes: u64,
    slots: u32,
    max_batch_items: u32,
    max_output_tokens: u32,
) -> ContextPlan {
    let slots = slots.max(1);
    let needed_per_slot = per_slot_tokens_needed(max_batch_items, max_output_tokens);
    let needed_total = round_up_1k(needed_per_slot as u64 * slots as u64);
    let baseline = baseline_context_for(ram_bytes) as u64;
    let mut ctx = needed_total.max(baseline);

    // None＝不知道記憶體或模型大小（不封頂）
    let ram_cap: Option<u64> = if ram_bytes == 0 || model_bytes == 0 {
        None
    } else {
        let free = ram_bytes.saturating_sub(model_bytes).saturating_sub(RAM_HEADROOM_BYTES);
        Some((free / kv_bytes_per_token(model_bytes)) / 1024 * 1024)
    };
    let mut tight = false;
    if let Some(ram_cap) = ram_cap.filter(|cap| ctx > *cap) {
        if ram_cap >= needed_total {
            ctx = ram_cap; // 保底值放不下，但需要量放得下：用放得下的最大值
        } else {
            tight = true; // 連需要量都放不下：照需要量開，日誌說明可能不穩
            ctx = needed_total;
        }
    }
    let ctx = ctx.min(131_072) as u32;
    ContextPlan {
        ctx,
        slots,
        per_slot: ctx / slots,
        needed_per_slot,
        ram_cap: ram_cap.unwrap_or(0).min(u32::MAX as u64) as u32,
        tight,
        reduced_from: 0,
    }
}

impl ContextPlan {
    /// 給使用者看的一段話（白話＋數字）。
    pub fn note(&self, ngl: u32) -> String {
        let mut s = format!(
            "本地模型啟動設定：上下文 {} 個字詞、同時處理 {} 個請求（每個約 {}，每批需要約 {}）、顯示卡層數 {}{}",
            self.ctx,
            self.slots,
            self.per_slot,
            self.needed_per_slot,
            ngl,
            if ngl == 0 { "（純 CPU，會比較慢）" } else { "" }
        );
        if self.reduced_from > 1 {
            s.push_str(&format!(
                "。記憶體估計只放得下約 {} 個字詞，已把同時處理數從 {} 降為 1（每個請求分到全部上下文）",
                self.ram_cap, self.reduced_from
            ));
        }
        if self.tight {
            s.push_str("。注意：記憶體仍不夠放下一批需要的量，可能被截斷或變慢；被截斷的批次會拆半重送");
        }
        s
    }
}

// ─── 等待上限 ─────────────────────────────────────────────────

/// 實測速度（token／秒 ×100，0＝還沒量到）。取指數平均，避免一批特別快或慢就大幅擺動。
static OBSERVED_TPS_X100: AtomicU64 = AtomicU64::new(0);

/// 記下一次實測速度（llama-server 回應的 `timings.predicted_per_second`）。
pub fn record_speed(tokens_per_second: f64) {
    let prev = OBSERVED_TPS_X100.load(Ordering::Relaxed);
    if let Some(next) = smooth_speed_x100(prev, tokens_per_second) {
        OBSERVED_TPS_X100.store(next, Ordering::Relaxed);
    }
}

/// 指數平均（純函式，方便測試）：`prev`＝0 表示還沒量過；無效樣本回 `None`。
fn smooth_speed_x100(prev: u64, tokens_per_second: f64) -> Option<u64> {
    if !tokens_per_second.is_finite() || tokens_per_second <= 0.0 {
        return None;
    }
    let sample = (tokens_per_second * 100.0) as u64;
    let next = if prev == 0 { sample } else { (prev * 3 + sample) / 4 };
    Some(next.max(1))
}

pub fn observed_speed() -> Option<f64> {
    match OBSERVED_TPS_X100.load(Ordering::Relaxed) {
        0 => None,
        v => Some(v as f64 / 100.0),
    }
}

/// 伺服器（重新）啟動時清掉上一次的實測速度（換了顯示卡層數、換了模型就不準了）。
/// 逾時延長倍數不在這裡：它跟著每次翻譯呼叫（見 `timeouts.rs`），新一輪一定從 ×1 開始。
pub fn reset_speed() {
    OBSERVED_TPS_X100.store(0, Ordering::Relaxed);
}

/// 「全部放進顯示卡」的層數門檻：安裝時的最高檔位用 99。
const FULL_OFFLOAD_NGL: u32 = 99;

/// 沒量到實測速度前，依顯示卡層數估速度（token／秒）。
///
/// `model_layers`：從 gguf 讀到的模型總層數（0＝讀不到）。
/// - 純 CPU（0 層）→ CPU 假設；全部放進顯示卡（層數 ≥ 總層數，或 99）→ GPU 假設；
/// - 部分放進：其餘層在 CPU 上跑、速度被拖住，取 CPU 與 GPU 之間、偏向 CPU 的值（依實際比例）；
/// - 讀不到總層數又不是全部放進：保守用 CPU 假設（待使用者實測）。
pub fn assumed_tps(ngl: u32, model_layers: u32) -> f64 {
    if ngl == 0 {
        return ASSUMED_TPS_CPU;
    }
    if ngl >= FULL_OFFLOAD_NGL || (model_layers > 0 && ngl >= model_layers) {
        return ASSUMED_TPS_GPU;
    }
    if model_layers == 0 {
        return ASSUMED_TPS_CPU;
    }
    let fraction = ngl as f64 / model_layers as f64;
    ASSUMED_TPS_CPU + (ASSUMED_TPS_GPU - ASSUMED_TPS_CPU) * fraction * 0.5
}

/// 一批最多等多久（秒）。
///
/// 估計這批的輸出量（每條約 60 token，不超過輸出上限），除以速度；
/// 同時處理 `slots` 個請求時運算是分著用的，所以再乘上 `slots`；
/// 加上讀提示的時間與固定餘裕，最後乘 2 當安全係數。夾在 [60, 900] 秒。
/// 有實測速度就用實測（打七折保守一點），沒有就用 GPU／CPU 的假設值（待使用者實測）。
///
/// `assumed_tps`：沒實測速度時的假設（[`assumed_tps`]）；
/// `stretch`：連續逾時後的延長倍數（`timeouts::TimeoutTracker`）；延長後仍不超過 900 秒。
pub fn request_timeout_secs(
    items: usize,
    max_output_tokens: usize,
    slots: u32,
    assumed_tps: f64,
    observed_tps: Option<f64>,
    stretch: f64,
) -> u64 {
    let tps = match observed_tps {
        Some(v) if v > 0.1 => v * 0.7,
        _ => assumed_tps.max(0.5),
    };
    let expected_output = (items.max(1) * TOKENS_PER_INPUT_ITEM as usize).min(max_output_tokens.max(1)) as f64;
    let generate = expected_output * slots.max(1) as f64 / tps;
    let prompt = 10.0 + items as f64 * 1.5;
    let secs = ((generate + prompt) * 2.0).ceil() as u64 + 15;
    let base = secs.clamp(MIN_TIMEOUT_SECS, MAX_TIMEOUT_SECS);
    ((base as f64 * stretch.max(1.0)).ceil() as u64).clamp(base, MAX_TIMEOUT_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn context_covers_every_slot_not_just_one() {
        // 16 GB、3 個 slot、一批 12 條、輸出上限 2048：每個請求要約 3924，三個就要 >11772
        let plan = plan_context(16 * GB, 7_400_000_000, 3, 12, 2048);
        assert!(plan.per_slot >= plan.needed_per_slot, "{plan:?}");
        assert!(plan.ctx >= plan.needed_per_slot * 3, "{plan:?}");
        assert!(!plan.tight);
    }

    #[test]
    fn context_never_drops_below_the_memory_baseline_when_it_fits() {
        let plan = plan_context(64 * GB, 7_400_000_000, 1, 12, 2048);
        assert_eq!(plan.ctx, 16384, "{plan:?}");
        assert!(plan.ctx >= 8192);
    }

    #[test]
    fn small_memory_is_flagged_tight_instead_of_silently_truncating() {
        // 8 GB 裝 7.4 GB 的模型：扣掉保留量幾乎沒有空間給 KV
        let plan = plan_context(8 * GB, 7_400_000_000, 2, 12, 2048);
        assert!(plan.tight, "{plan:?}");
        assert_eq!(plan.slots, 1, "先降成一次處理一個：{plan:?}");
        assert!(plan.ctx >= plan.needed_per_slot, "照需要量開，交給拆批處理：{plan:?}");
        assert!(plan.note(0).contains("拆半"));
    }

    #[test]
    fn fix1c_tight_memory_drops_to_one_slot_and_says_so() {
        let plan = plan_context(10 * GB, 7_400_000_000, 3, 12, 2048);
        assert_eq!(plan.slots, 1, "記憶體不夠時先降成一次處理一個請求：{plan:?}");
        assert!(plan.per_slot >= plan.needed_per_slot.min(plan.ctx), "{plan:?}");
        let note = plan.note(0);
        assert!(note.contains("降為 1"), "日誌要照實寫：{note}");
    }

    #[test]
    fn plan_note_is_plain_language_with_numbers() {
        let plan = plan_context(32 * GB, 7_400_000_000, 3, 12, 2048);
        let note = plan.note(0);
        assert!(note.contains("上下文") && note.contains("同時處理 3 個請求") && note.contains("純 CPU"), "{note}");
    }

    #[test]
    fn timeout_grows_with_batch_size_and_shrinks_with_speed() {
        let cpu_big = request_timeout_secs(12, 2048, 2, assumed_tps(0, 0), None, 1.0);
        let cpu_small = request_timeout_secs(3, 2048, 2, assumed_tps(0, 0), None, 1.0);
        let gpu_big = request_timeout_secs(12, 2048, 2, assumed_tps(99, 0), None, 1.0);
        assert!(cpu_big > cpu_small, "{cpu_big} {cpu_small}");
        assert!(cpu_big > gpu_big, "{cpu_big} {gpu_big}");
        // 純 CPU 一批 12 條不能再用舊的 90 秒上限（實測會一直逾時空轉）
        assert!(cpu_big > 90, "{cpu_big}");
        for items in [1usize, 6, 12, 48] {
            for gpu in [true, false] {
                let t = request_timeout_secs(items, 2048, 3, assumed_tps(if gpu { 99 } else { 0 }, 0), None, 1.0);
                assert!((MIN_TIMEOUT_SECS..=MAX_TIMEOUT_SECS).contains(&t), "{items} {gpu} {t}");
            }
        }
        // 有實測速度就照實測
        let fast = request_timeout_secs(12, 2048, 1, assumed_tps(0, 0), Some(80.0), 1.0);
        assert!(fast < cpu_big, "{fast} {cpu_big}");
    }

    #[test]
    fn fix1a_partial_offload_is_not_assumed_to_run_at_gpu_speed() {
        assert_eq!(assumed_tps(0, 36), ASSUMED_TPS_CPU);
        assert_eq!(assumed_tps(99, 0), ASSUMED_TPS_GPU);
        assert_eq!(assumed_tps(40, 36), ASSUMED_TPS_GPU, "層數夠多＝全部放進顯示卡");
        let low = assumed_tps(20, 36);
        assert!(low > ASSUMED_TPS_CPU && low < ASSUMED_TPS_GPU * 0.75, "20／36 層仍被 CPU 拖住：{low}");
        let t_low = request_timeout_secs(12, 2048, 2, low, None, 1.0);
        let t_full = request_timeout_secs(12, 2048, 2, assumed_tps(99, 36), None, 1.0);
        assert!(t_low > t_full, "{t_low} {t_full}");
    }

    #[test]
    fn fix2_f7_unknown_layer_count_is_assumed_cpu_speed() {
        assert_eq!(assumed_tps(20, 0), ASSUMED_TPS_CPU, "讀不到總層數：保守當 CPU");
        assert_eq!(assumed_tps(40, 0), ASSUMED_TPS_CPU);
    }

    #[test]
    fn fix1a_repeated_timeouts_stretch_the_wait_up_to_the_cap() {
        let tps = assumed_tps(99, 0);
        let base = request_timeout_secs(6, 2048, 1, tps, None, 1.0);
        let longer = request_timeout_secs(6, 2048, 1, tps, None, 1.5);
        assert!(longer > base, "{base} {longer}");
        assert_eq!(request_timeout_secs(12, 2048, 3, assumed_tps(0, 0), None, 8.0), MAX_TIMEOUT_SECS, "延長也不超過 900 秒");
    }

    #[test]
    fn speed_samples_are_smoothed() {
        // 純函式測試，不碰全域狀態（審查 F3：避免與平行測試互相干擾）
        let first = smooth_speed_x100(0, 10.0).unwrap();
        let second = smooth_speed_x100(first, 30.0).unwrap();
        assert!(second > first && second < 3000, "{first} {second}");
        assert_eq!(smooth_speed_x100(second, f64::NAN), None);
        assert_eq!(smooth_speed_x100(second, -1.0), None);
    }
}
