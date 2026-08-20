//! 開發進度偵測：與玩家 UI 日誌分離，寫入工作根「開發進度偵測.txt」。
//! 每次任務開始覆寫；結束附階段耗時排行，方便找瓶頸。

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Instant;

pub const DEV_TRACE_FILE: &str = "開發進度偵測.txt";
const MAX_BYTES: usize = 512 * 1024;

pub const STAGE_PREP: &str = "prep";
pub const STAGE_SCAN: &str = "scan";
pub const STAGE_LOCAL: &str = "local";
pub const STAGE_TRANSLATE: &str = "translate";
pub const STAGE_EXTRAS: &str = "extras";
pub const STAGE_PACKAGE: &str = "package";
pub const STAGE_APPLY: &str = "apply";
pub const STAGE_TOTAL: u8 = 7;
pub const UI_STEP_TOTAL: u8 = 5;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageProgressSpec {
    pub name: &'static str,
    pub step: u8,
    pub base_percent: u8,
    pub span_percent: u8,
}

const STAGE_SPECS: [StageProgressSpec; STAGE_TOTAL as usize] = [
    StageProgressSpec {
        name: STAGE_PREP,
        step: 1,
        base_percent: 0,
        span_percent: 0,
    },
    StageProgressSpec {
        name: STAGE_SCAN,
        step: 2,
        base_percent: 0,
        span_percent: 15,
    },
    StageProgressSpec {
        name: STAGE_LOCAL,
        step: 2,
        base_percent: 15,
        span_percent: 5,
    },
    StageProgressSpec {
        name: STAGE_TRANSLATE,
        step: 3,
        base_percent: 20,
        span_percent: 50,
    },
    StageProgressSpec {
        name: STAGE_EXTRAS,
        step: 4,
        base_percent: 70,
        span_percent: 20,
    },
    StageProgressSpec {
        name: STAGE_PACKAGE,
        step: 4,
        base_percent: 90,
        span_percent: 7,
    },
    StageProgressSpec {
        name: STAGE_APPLY,
        step: 5,
        base_percent: 97,
        span_percent: 3,
    },
];

pub fn stage_progress_spec(stage: &str) -> Option<StageProgressSpec> {
    STAGE_SPECS.iter().copied().find(|spec| spec.name == stage)
}

pub fn weighted_percent(stage: &str, done: u64, total: u64) -> Option<u8> {
    let spec = stage_progress_spec(stage)?;
    if total == 0 {
        return Some(spec.base_percent);
    }
    let clamped_done = done.min(total);
    let span = (clamped_done as u128 * spec.span_percent as u128) / total as u128;
    Some((spec.base_percent as u128 + span).min(100) as u8)
}

#[derive(Debug, Clone)]
struct StageRec {
    name: String,
    ms: u128,
}

#[derive(Debug)]
struct TraceInner {
    _path: PathBuf,
    t0: Instant,
    lines: Vec<String>,
    open_stages: Vec<(String, Instant)>,
    finished: Vec<StageRec>,
    truncated: bool,
}

static TRACE: Mutex<Option<TraceInner>> = Mutex::new(None);

fn now_stamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    // 簡短 UTC（與錯誤日誌同風格，不額外依賴 chrono
    let days = secs / 86_400;
    let tod = secs % 86_400;
    let (h, mi, s) = (tod / 3600, (tod % 3600) / 60, tod % 60);
    format!("day{days}+{h:02}:{mi:02}:{s:02}Z")
}

fn push_line(inner: &mut TraceInner, kind: &str, detail: &str) {
    let elapsed = inner.t0.elapsed().as_millis();
    let line = format!(
        "{}\t+{}ms\t{}\t{}",
        now_stamp(),
        elapsed,
        kind,
        detail.replace('\t', " ").replace('\n', " ")
    );
    inner.lines.push(line);
    let approx: usize = inner.lines.iter().map(|l| l.len() + 1).sum();
    if approx > MAX_BYTES {
        inner.truncated = true;
        let keep_head = 40usize.min(inner.lines.len());
        let keep_tail = 200usize.min(inner.lines.len().saturating_sub(keep_head));
        let mut kept = Vec::with_capacity(keep_head + keep_tail + 1);
        kept.extend_from_slice(&inner.lines[..keep_head]);
        kept.push("…（中段已截斷以控制檔案大小）…".into());
        let start = inner.lines.len() - keep_tail;
        kept.extend_from_slice(&inner.lines[start..]);
        inner.lines = kept;
    }
}

fn flush_unlocked(_inner: &TraceInner) {
    // 1.0.2+：不再寫入工作根「開發進度偵測.txt」，翻譯結果只留玩家可用產物。
}

/// 開始一輪偵測（覆寫舊檔）。
pub fn start(work: &Path) {
    let path = work.join(DEV_TRACE_FILE);
    let mut guard = TRACE.lock().unwrap_or_else(|e| e.into_inner());
    let mut inner = TraceInner {
        _path: path,
        t0: Instant::now(),
        lines: Vec::new(),
        open_stages: Vec::new(),
        finished: Vec::new(),
        truncated: false,
    };
    push_line(&mut inner, "START", &format!("work={}", work.display()));
    flush_unlocked(&inner);
    *guard = Some(inner);
}

pub fn mark(detail: &str) {
    let mut guard = TRACE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(inner) = guard.as_mut() else {
        return;
    };
    push_line(inner, "MARK", detail);
}

pub fn enter(stage: &str) {
    let mut guard = TRACE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(inner) = guard.as_mut() else {
        return;
    };
    push_line(inner, "ENTER", stage);
    inner.open_stages.push((stage.to_string(), Instant::now()));
}

pub fn leave(stage: &str) {
    let mut guard = TRACE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(inner) = guard.as_mut() else {
        return;
    };
    let mut ms = 0u128;
    if let Some(pos) = inner
        .open_stages
        .iter()
        .rposition(|(name, _)| name == stage)
    {
        let (_, t) = inner.open_stages.remove(pos);
        ms = t.elapsed().as_millis();
        inner.finished.push(StageRec {
            name: stage.to_string(),
            ms,
        });
    }
    push_line(inner, "LEAVE", &format!("{stage} ({ms}ms)"));
}

/// 結束並寫耗時排行。
pub fn finish(outcome: &str) {
    let mut guard = TRACE.lock().unwrap_or_else(|e| e.into_inner());
    let Some(mut inner) = guard.take() else {
        return;
    };
    // 未關閉的階段一律 leave
    while let Some((name, t)) = inner.open_stages.pop() {
        let ms = t.elapsed().as_millis();
        inner.finished.push(StageRec {
            name: name.clone(),
            ms,
        });
        push_line(&mut inner, "LEAVE", &format!("{name} ({ms}ms, auto)"));
    }
    let total = inner.t0.elapsed().as_millis();
    push_line(&mut inner, "FINISH", &format!("outcome={outcome} total={total}ms"));
    let mut ranked = inner.finished.clone();
    ranked.sort_by(|a, b| b.ms.cmp(&a.ms));
    push_line(&mut inner, "SUMMARY", "—— 階段耗時排行（長→短）——");
    for (i, rec) in ranked.iter().take(40).enumerate() {
        push_line(
            &mut inner,
            "RANK",
            &format!("{}. {} — {}ms", i + 1, rec.name, rec.ms),
        );
    }
    flush_unlocked(&inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::fs;

    #[test]
    fn start_leave_finish_keeps_trace_in_memory_only() {
        let dir = env::temp_dir().join(format!("dev_trace_{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        start(&dir);
        enter("scan");
        mark("jars=3");
        leave("scan");
        finish("ok");
        assert!(
            !dir.join(DEV_TRACE_FILE).exists(),
            "1.0.2+ 不再寫入開發進度偵測.txt"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn weighted_percent_is_monotonic_across_stages() {
        let stages = [
            (STAGE_SCAN, 10u64),
            (STAGE_LOCAL, 10u64),
            (STAGE_TRANSLATE, 20u64),
            (STAGE_EXTRAS, 10u64),
            (STAGE_PACKAGE, 10u64),
            (STAGE_APPLY, 10u64),
        ];
        let mut last = 0u8;
        for (stage, total) in stages {
            for done in 0..=total {
                let pct = weighted_percent(stage, done, total).unwrap();
                assert!(pct >= last, "{stage} {done}/{total} regressed from {last} to {pct}");
                last = pct;
            }
        }
        assert_eq!(last, 100);
    }

    #[test]
    fn weighted_percent_respects_stage_boundaries() {
        let scan = stage_progress_spec(STAGE_SCAN).unwrap();
        let translate = stage_progress_spec(STAGE_TRANSLATE).unwrap();
        let apply = stage_progress_spec(STAGE_APPLY).unwrap();
        assert_eq!(weighted_percent(STAGE_SCAN, 0, 10), Some(scan.base_percent));
        assert_eq!(
            weighted_percent(STAGE_SCAN, 10, 10),
            Some(scan.base_percent + scan.span_percent)
        );
        assert_eq!(weighted_percent(STAGE_TRANSLATE, 0, 10), Some(translate.base_percent));
        assert_eq!(
            weighted_percent(STAGE_TRANSLATE, 10, 10),
            Some(translate.base_percent + translate.span_percent)
        );
        assert_eq!(
            weighted_percent(STAGE_APPLY, 10, 10),
            Some(apply.base_percent + apply.span_percent)
        );
    }

    #[test]
    fn weighted_percent_is_safe_when_total_zero() {
        assert_eq!(weighted_percent(STAGE_SCAN, 0, 0), Some(0));
        assert_eq!(weighted_percent(STAGE_LOCAL, 3, 0), Some(15));
        assert_eq!(weighted_percent(STAGE_APPLY, 9, 0), Some(97));
        assert_eq!(weighted_percent("unknown", 1, 1), None);
    }
}
