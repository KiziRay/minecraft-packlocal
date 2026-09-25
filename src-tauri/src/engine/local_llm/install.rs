use super::download::{download_file_resumable, fetch_manifest_text, skip_if_sha_matches};
use super::hardware::probe_hardware;
use super::manifest::parse_manifest_json;
use super::select::{
    bytes_needed_for_plan, fallback_runtime, plan_from_manifest, zip_for, HwFacts, InstallPlan,
};
use super::server::{files_ready, remember_gguf_path, remember_install_dir};
use crate::engine::disk::ensure_space;
use crate::engine::security::is_safe_zip_entry_name;
use serde::Serialize;
use std::fs::{self, File};
use std::io::copy;
use std::path::{Path, PathBuf};
use zip::ZipArchive;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProbeView {
    pub ram_bytes: u64,
    pub cpu_cores: u32,
    pub gpu_name: String,
    pub vram_bytes: u64,
    pub nvidia: bool,
    pub runtime: String,
    pub profile: String,
    pub gguf_name: String,
    pub zip_name: String,
    pub need_bytes: u64,
    pub ready: bool,
    pub install_dir: String,
    pub local_model_found: bool,
    /// 是否已經拿到遠端清單。false＝檔名／大小全部未知，前端**不得**顯示任何下載大小。
    pub plan_known: bool,
    pub message: String,
    /// 記住的安裝資料夾已經不存在時的說明；空字串＝沒有這回事。
    ///
    /// 舊版遇到這情況會靜默改用預設目錄重新開始，使用者只會納悶「怎麼又要重新下載」。
    #[serde(default)]
    pub moved_notice: String,
}

/// 新安裝的預設落點：跟其餘子系統統一放可攜式根（見 server.rs 的 `state_path` 註解）。
/// 只影響「還沒裝過」的情況——`resolve_install_dir` 一律先看 `remembered_install_dir()`，
/// 既有安裝記得的路徑（不論在哪個根目錄下）永遠優先，這裡只是空白時的預設值。
pub(crate) fn default_install_dir() -> PathBuf {
    super::super::paths::portable_root().join("local-llm")
}

/// 決定本地模型的安裝資料夾。
///
/// 呼叫端沒有指定時，**必須先用記住的目錄**（`server.json` 的 `installDir`），
/// 再退回預設目錄。舊版直接跳到預設目錄，於是同一份程式對「裝在哪」有兩種答案：
///   `is_installed()`／`status_view()` 讀 state → 說「已安裝」
///   `resolve_install_dir(None)`        → 看預設目錄 → 說「尚未安裝」
/// 使用者把模型裝到 D:\bot\mo 之後，翻譯就會在 63% 卡住並回報「尚未安裝本地模型」，
/// 而 7.38 GB 的模型其實好好待在那裡。兩句錯誤訊息還一模一樣，畫面上看不出來。
pub fn resolve_install_dir(chosen: Option<&str>) -> Result<PathBuf, String> {
    let raw = chosen.map(str::trim).filter(|s| !s.is_empty());
    let path = if let Some(s) = raw {
        crate::engine::security::normalize_user_path(s)?
    } else {
        remembered_install_dir().unwrap_or_else(default_install_dir)
    };
    fs::create_dir_all(&path).map_err(|e| format!("無法建立本地模型資料夾：{e}"))?;
    Ok(path)
}

/// `server.json` 記住的安裝目錄；空字串或不存在時回 None。
pub(crate) fn remembered_install_dir() -> Option<PathBuf> {
    let state = super::server::load_state();
    let dir = state.install_dir.trim();
    if dir.is_empty() {
        return None;
    }
    let path = PathBuf::from(dir);
    if path.is_dir() {
        Some(path)
    } else {
        None
    }
}

/// 記住的安裝目錄如果已經不存在，回傳它原本的路徑；否則回 `None`。
///
/// 必須在 `remember_install_dir()` 把新路徑寫回 state **之前**呼叫，
/// 不然舊路徑會被蓋掉，永遠偵測不到「搬移過」這件事。
fn remembered_dir_missing() -> Option<String> {
    let state = super::server::load_state();
    let dir = state.install_dir.trim();
    if dir.is_empty() || Path::new(dir).is_dir() {
        None
    } else {
        Some(dir.to_string())
    }
}

fn player_probe_message(facts: &HwFacts, need_bytes: u64, local_model: bool) -> String {
    let mb = need_bytes / (1024 * 1024);
    if local_model {
        return format!("這個資料夾裡已有先前下載並驗證過的翻譯模型，這次只需要補齊其餘檔案（約 {mb} MB，含解壓餘裕）。");
    }
    if facts.has_gpu && !facts.gpu_name.trim().is_empty() {
        format!("已依這台電腦選擇一套翻譯模型與執行程式（約 {mb} MB，含解壓餘裕）。將用顯示卡加速翻譯。")
    } else {
        format!("未偵測到可用顯示卡，將改用處理器執行（約 {mb} MB，含解壓餘裕）。速度會比較慢。")
    }
}

/// 只認「安裝資料夾內、且雜湊與遠端清單相符」的既有模型。
///
/// 舊版曾掃描機器上任意 `.gguf`（含開發機硬編碼路徑）並直接沿用，等於跳過 SHA256——
/// 任何來路不明的權重都會被載入去產生玩家看到的譯文。這裡改成唯一判準：檔名、大小、
/// 雜湊三者都對得上遠端清單才算數，對不上就重新下載。
fn verified_gguf_in_install_dir(install_dir: &Path, plan: &InstallPlan) -> Option<PathBuf> {
    let candidate = install_dir.join("models").join(&plan.gguf.name);
    match skip_if_sha_matches(&candidate, &plan.gguf) {
        Ok(true) => Some(candidate),
        _ => None,
    }
}

fn human_bytes(bytes: u64) -> String {
    const GB: u64 = 1024 * 1024 * 1024;
    const MB: u64 = 1024 * 1024;
    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else {
        format!("{} MB", bytes / MB)
    }
}

fn human_duration(secs: u64) -> String {
    if secs < 60 {
        format!("約 {secs} 秒")
    } else if secs < 3600 {
        format!("約 {} 分鐘", (secs + 59) / 60)
    } else {
        format!("約 {} 小時 {} 分鐘", secs / 3600, (secs % 3600) / 60)
    }
}

/// 「下載翻譯模型… 1.2/7.4 GB · 18.5 MB/s · 剩約 6 分鐘」
fn download_progress_line(done: u64, total: u64, elapsed: std::time::Duration) -> String {
    let secs = elapsed.as_secs_f64().max(0.001);
    let rate = done as f64 / secs;
    let mut line = String::from("下載翻譯模型…");
    if total > 0 {
        line.push_str(&format!(" {}/{}", human_bytes(done), human_bytes(total)));
    }
    if rate > 1024.0 {
        line.push_str(&format!(" · {:.1} MB/s", rate / (1024.0 * 1024.0)));
        if total > done {
            let remain = ((total - done) as f64 / rate) as u64;
            line.push_str(&format!(" · 剩{}", human_duration(remain)));
        }
    }
    line
}

pub(crate) fn ngl_for_profile(profile: super::select::HwProfile) -> u32 {
    match profile {
        super::select::HwProfile::High => 99,
        super::select::HwProfile::Med => 40,
        super::select::HwProfile::Low => 20,
        super::select::HwProfile::Cpu => 0,
    }
}

fn ngl_for(plan: &InstallPlan) -> u32 {
    ngl_for_profile(plan.profile)
}

/// 已安裝且已啟動時回報的狀態；不需要遠端清單就能組出來。
fn installed_probe_view(facts: &HwFacts, install_dir: &Path) -> ProbeView {
    ProbeView {
        ram_bytes: facts.ram_bytes,
        cpu_cores: facts.cpu_cores,
        gpu_name: facts.gpu_name.clone(),
        vram_bytes: facts.vram_bytes,
        nvidia: facts.nvidia,
        runtime: String::new(),
        profile: String::new(),
        gguf_name: String::new(),
        zip_name: String::new(),
        need_bytes: 0,
        ready: true,
        install_dir: install_dir.display().to_string(),
        local_model_found: true,
        plan_known: false,
        message: "本地模型已安裝並就緒，不需要重新下載。".into(),
        moved_notice: String::new(),
    }
}

pub fn build_probe_view(facts: &HwFacts, plan: &InstallPlan, install_dir: &Path) -> ProbeView {
    let local = verified_gguf_in_install_dir(install_dir, plan).is_some();
    let need = if local {
        plan.zip.bytes.saturating_add(super::select::SPACE_MARGIN_BYTES)
    } else {
        bytes_needed_for_plan(plan)
    };
    ProbeView {
        ram_bytes: facts.ram_bytes,
        cpu_cores: facts.cpu_cores,
        gpu_name: facts.gpu_name.clone(),
        vram_bytes: facts.vram_bytes,
        nvidia: facts.nvidia,
        runtime: plan.runtime.as_str().into(),
        profile: plan.profile.as_str().into(),
        gguf_name: plan.gguf.name.clone(),
        zip_name: plan.zip.name.clone(),
        need_bytes: need,
        ready: files_ready(install_dir),
        install_dir: install_dir.display().to_string(),
        local_model_found: local,
        plan_known: true,
        message: player_probe_message(facts, need, local),
        moved_notice: String::new(),
    }
}

/// 還沒登入 Discord 時也能看到的「這台電腦適不適合」報告。
///
/// 遠端清單需要會籍，所以檔名／大小這一段留空；使用者先看硬體結論再決定要不要登入。
fn hardware_only_probe_view(facts: &HwFacts, install_dir: &Path, gate_hint: &str) -> ProbeView {
    ProbeView {
        ram_bytes: facts.ram_bytes,
        cpu_cores: facts.cpu_cores,
        gpu_name: facts.gpu_name.clone(),
        vram_bytes: facts.vram_bytes,
        nvidia: facts.nvidia,
        runtime: String::new(),
        profile: String::new(),
        gguf_name: String::new(),
        zip_name: String::new(),
        need_bytes: 0,
        ready: false,
        install_dir: install_dir.display().to_string(),
        local_model_found: false,
        plan_known: false,
        message: gate_hint.to_string(),
        moved_notice: String::new(),
    }
}

/// 下載並解壓執行程式（約 35 MB）。
///
/// 這是安裝流程裡最便宜的一步，所以被排到最前面：解壓完立刻實跑一次就能判定
/// 這台機器行不行，不必先付 7.38 GB 的下載代價。換 runtime 時也只重下這 35 MB。
fn prepare_runtime(
    install_dir: &Path,
    plan: &InstallPlan,
    on_progress: &mut dyn FnMut(u8, &str),
) -> Result<(), String> {
    let runtime = install_dir.join("runtime");
    let zip_path = install_dir.join(&plan.zip.name);
    download_file_resumable(&plan.zip, &zip_path, &mut |done, total| {
        let pct = if total == 0 { 16 } else { 12 + ((done * 8) / total.max(1)) as u8 };
        on_progress(pct.min(20), "下載執行程式…");
    })?;
    on_progress(22, "解壓執行程式…");
    swap_in_runtime(&runtime, &zip_path)
}

/// 解壓到暫存目錄，成功後才換掉現行 runtime（P1-06）。
///
/// 舊版是先 `remove_dir_all(runtime)` 再解壓：解壓中途失敗（壞 zip、磁碟滿、
/// 防毒攔截）就把原本**可以用**的 runtime 一起毀了，使用者從「有一個舊版能跑」
/// 變成「兩個都沒有」。更新失敗的正確結果是維持原狀，不是變得更糟。
fn swap_in_runtime(runtime: &Path, zip_path: &Path) -> Result<(), String> {
    let staging = sibling_dir(runtime, ".staging");
    let previous = sibling_dir(runtime, ".previous");

    let _ = fs::remove_dir_all(&staging);
    fs::create_dir_all(&staging).map_err(|e| format!("無法建立暫存目錄：{e}"))?;

    // 解壓失敗就地清掉暫存並回報；現行 runtime 完全沒被碰過。
    if let Err(e) = extract_zip(zip_path, &staging) {
        let _ = fs::remove_dir_all(&staging);
        return Err(e);
    }

    // 換裝：舊的先挪到 .previous，新的就位；任一步失敗都要能回到原狀。
    let _ = fs::remove_dir_all(&previous);
    if runtime.exists() {
        if let Err(e) = fs::rename(runtime, &previous) {
            let _ = fs::remove_dir_all(&staging);
            return Err(format!("無法備份現有執行程式：{e}"));
        }
    }
    if let Err(e) = fs::rename(&staging, runtime) {
        // 新的裝不上去就把舊的放回來，不留下一個空的 runtime
        if previous.exists() {
            let _ = fs::rename(&previous, runtime);
        }
        let _ = fs::remove_dir_all(&staging);
        return Err(format!("無法啟用新的執行程式：{e}"));
    }

    // 到這裡新版已就位，舊版才可以刪。刪不掉不算失敗（下次會被覆蓋）。
    let _ = fs::remove_dir_all(&previous);
    Ok(())
}

fn sibling_dir(base: &Path, suffix: &str) -> PathBuf {
    let mut name = base.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    base.with_file_name(name)
}

fn extract_zip(zip_path: &Path, dest: &Path) -> Result<(), String> {
    fs::create_dir_all(dest).map_err(|e| e.to_string())?;
    let file = File::open(zip_path).map_err(|_| "無法開啟執行套件。".to_string())?;
    let mut archive = ZipArchive::new(file).map_err(|_| "執行套件無效。".to_string())?;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(|e| e.to_string())?;
        let name = entry.name().replace('\\', "/");
        if !is_safe_zip_entry_name(&name) {
            continue;
        }
        let out = dest.join(name);
        if entry.is_dir() {
            fs::create_dir_all(&out).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
        let mut outfile = File::create(&out).map_err(|e| e.to_string())?;
        copy(&mut entry, &mut outfile).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 偵測＝本機動作，不該先要求登入。
///
/// 硬體結論一定給得出來；只有「要下幾 GB」需要遠端清單（需會籍）。拿不到清單時回傳
/// 硬體版報告＋一句可執行的下一步，讓使用者先知道自己的電腦適不適合再決定要不要登入。
pub fn probe_install(chosen_dir: Option<&str>) -> Result<ProbeView, String> {
    // 只有「沒有明確指定資料夾」時才是走記住的路徑，才需要檢查它是否被搬走；
    // 使用者自己選了一個資料夾，不管那是不是新的，都不算「搬移」。
    let explicit = chosen_dir.map(str::trim).filter(|s| !s.is_empty()).is_some();
    let moved_from = if explicit { None } else { remembered_dir_missing() };
    let install_dir = resolve_install_dir(chosen_dir)?;
    remember_install_dir(&install_dir);
    super::server::forget_foreign_gguf_pin(&install_dir);
    let facts = probe_hardware();
    let mut view = match fetch_manifest_text() {
        Ok(text) => {
            let manifest = parse_manifest_json(&text)?;
            let plan = plan_from_manifest(&facts, &manifest)?;
            build_probe_view(&facts, &plan, &install_dir)
        }
        Err(gate) => hardware_only_probe_view(&facts, &install_dir, &gate),
    };
    if let Some(old) = moved_from {
        view.moved_notice = format!(
            "找不到之前設定的資料夾（{old}），可能已被搬移、改名或刪除。已改用新的位置（{}），需要重新準備檔案。",
            install_dir.display()
        );
    }
    Ok(view)
}

/// 本機檔案是不是「目前這版清單」認可的模型。
///
/// `files_ready()` 只看「檔名合理、大小夠大」，從不比對雜湊——R2 換了新模型之後，
/// 舊檔案會永遠通過那個檢查，快速路徑因此永遠不會觸發重新下載，使用者用著過期模型
/// 卻毫無異狀。這裡才是真正核對雜湊的地方；`None` 代表清單抓不到（離線／未登入），
/// 呼叫端要自己決定失效方向——這裡選擇信任本機檔案，不能因為連不上就擋住離線可用性。
fn manifest_says_current(install_dir: &Path, facts: &HwFacts) -> Option<bool> {
    let text = fetch_manifest_text().ok()?;
    let manifest = parse_manifest_json(&text).ok()?;
    let plan = plan_from_manifest(facts, &manifest).ok()?;
    Some(verified_gguf_in_install_dir(install_dir, &plan).is_some())
}

pub fn install_and_start(
    chosen_dir: Option<&str>,
    on_progress: &mut dyn FnMut(u8, &str),
) -> Result<ProbeView, String> {
    let explicit = chosen_dir.map(str::trim).filter(|s| !s.is_empty()).is_some();
    let moved_from = if explicit { None } else { remembered_dir_missing() };
    let install_dir = resolve_install_dir(chosen_dir)?;
    if let Some(old) = &moved_from {
        on_progress(
            2,
            &format!("找不到之前設定的資料夾（{old}），改用新的位置重新準備。"),
        );
    }
    remember_install_dir(&install_dir);
    super::server::forget_foreign_gguf_pin(&install_dir);
    let facts = probe_hardware();
    // 已經裝好就別再走一次完整下載流程。舊版無條件抓遠端清單＋算空間，沒登入時連
    // 「檔案早就在本機」都會失敗，體感像每次都要重裝——但完全略過清單比對又會讓
    // 換版本的模型永遠偵測不到，兩害相權，這裡先花一次便宜的清單查詢核對版本。
    if files_ready(&install_dir) {
        let up_to_date = manifest_says_current(&install_dir, &facts).unwrap_or(true);
        if up_to_date {
            on_progress(60, "這台電腦已經裝好本地模型，正在啟動…");
            let runtime = super::select::choose_runtime(&facts);
            let ngl = ngl_for_profile(super::select::choose_profile(&facts, runtime));
            match super::server::start_server_with_fallback(&install_dir, ngl, |_, note| {
                on_progress(80, note);
            }) {
                Ok(_) => {
                    on_progress(100, "本地模型已就緒，可以按開始翻譯。");
                    return Ok(installed_probe_view(&facts, &install_dir));
                }
                // 起不來才回頭走完整安裝（可能是檔案壞了或 runtime 不相容）
                Err(err) => on_progress(10, &format!("既有安裝無法啟動（{err}），重新準備檔案…")),
            }
        } else {
            on_progress(6, "偵測到雲端已更新翻譯模型，準備下載新版本…");
        }
    }
    crate::engine::discord_auth::require_discord_guild_for_ai()?;
    on_progress(8, "已偵測這台電腦，正在計算需要的檔案…");
    let text = fetch_manifest_text()?;
    let manifest = parse_manifest_json(&text)?;
    let mut plan = plan_from_manifest(&facts, &manifest)?;
    if plan.files().iter().any(|f| f.name.trim().is_empty()) {
        return Err("遠端清單缺少模型或執行套件檔名。".into());
    }
    let models = install_dir.join("models");
    let runtime = install_dir.join("runtime");
    fs::create_dir_all(&models).map_err(|e| e.to_string())?;
    fs::create_dir_all(&runtime).map_err(|e| e.to_string())?;

    // ── 階段零：記憶體前置檢查。不夠就當場說清楚，不要下載完 7.38 GB 才失敗 ───────
    super::select::preflight(&facts, plan.gguf.bytes)?;

    // ── 階段一：先把 35 MB 的執行程式弄好並「實跑一次」確認這台機器相容 ──────────
    //
    // 舊版順序是「先下 7.38 GB 模型 → 再下 35 MB 執行程式 → 才知道跑不跑得動」。
    // 執行程式不相容的機器（顯示卡驅動太舊、處理器缺指令集）要付出整包下載的代價
    // 才會發現不行。倒過來之後，35 MB 就能判定，而且換一套 runtime 也只重下 35 MB。
    ensure_space(&install_dir, plan.zip.bytes.saturating_add(super::select::SPACE_MARGIN_BYTES))?;
    on_progress(12, "下載執行程式（約 35 MB）…");
    prepare_runtime(&install_dir, &plan, &mut *on_progress)?;
    on_progress(24, "檢查這台電腦能不能執行…");
    while let Err(reason) = super::server::runtime_can_run(&install_dir) {
        let Some(next) = fallback_runtime(plan.runtime) else {
            return Err(format!(
                "這台電腦無法執行任何一套本地模型程式（{reason}）。
                 常見原因是顯示卡驅動過舊或處理器不支援。可以改用自訂 API 或 GPT 翻譯。"
            ));
        };
        on_progress(16, "這套執行方式在這台電腦上跑不起來，改用較相容的一套…");
        plan.runtime = next;
        plan.zip = zip_for(&manifest, next);
        if plan.zip.name.trim().is_empty() {
            return Err(format!("這台電腦無法執行本地模型（{reason}），也沒有其他可用的執行程式。"));
        }
        ensure_space(&install_dir, plan.zip.bytes.saturating_add(super::select::SPACE_MARGIN_BYTES))?;
        prepare_runtime(&install_dir, &plan, &mut *on_progress)?;
    }

    // ── 階段二：確認相容之後才下載模型本體 ───────────────────────────────────
    let need = if verified_gguf_in_install_dir(&install_dir, &plan).is_some() {
        super::select::SPACE_MARGIN_BYTES
    } else {
        plan.gguf.bytes.saturating_add(super::select::SPACE_MARGIN_BYTES)
    };
    ensure_space(&install_dir, need)?;
    // 一律走 download_file_resumable：它內建「雜湊相符就略過」，所以既有檔案仍會被重用，
    // 但重用的前提永遠是驗證通過，不存在跳過 SHA256 的捷徑。
    let gguf_path = models.join(&plan.gguf.name);
    on_progress(30, "下載翻譯模型…");
    let started = std::time::Instant::now();
    let mut last_emit = std::time::Instant::now();
    download_file_resumable(&plan.gguf, &gguf_path, &mut |done, total| {
        let pct = if total == 0 {
            50
        } else {
            30 + ((done * 45) / total.max(1)) as u8
        };
        // 幾 GB 的下載沒有速率與剩餘時間，使用者只會看到一條幾乎不動的進度條。
        // 每秒最多更新一次，避免每 1 MiB 就打一次事件。
        if last_emit.elapsed() >= std::time::Duration::from_millis(900) {
            last_emit = std::time::Instant::now();
            on_progress(pct.min(75), &download_progress_line(done, total, started.elapsed()));
        }
    })?;
    remember_gguf_path(&gguf_path);

    // ── 階段三：啟動，並且「真的翻一句」才算就緒 ──────────────────────────────
    on_progress(80, "啟動本地模型…");
    let (port, _) = super::server::start_server_with_fallback(&install_dir, ngl_for(&plan), |_, note| {
        on_progress(85, note);
    })?;
    on_progress(92, "試翻一句話，確認真的可以用…");
    super::server::verify_can_translate(port)?;

    on_progress(100, "本地模型可以使用，可以按開始翻譯。");
    Ok(build_probe_view(&facts, &plan, &install_dir))
}

/// 翻譯當下確保服務可用。
///
/// 舊版只算一次 ngl 就 `ensure_running`，起不來就直接失敗——所以會出現「檔案明明都在，
/// 一按開始翻譯就說本地模型沒有回應」。現在共用與安裝時同一套 ngl 階梯降級：
/// 顯示記憶體不夠就少放幾層，最後保底純 CPU。任何裝得起來的機器都要能翻。
pub fn ensure_ready_for_translate(install_dir: Option<&str>) -> Result<u16, String> {
    crate::engine::discord_auth::require_discord_guild_for_ai()?;
    let dir = resolve_install_dir(install_dir)?;
    if !files_ready(&dir) {
        // 刻意與「尚未安裝」用不同句子：這是「裝過但這個資料夾裡東西不齊／找錯地方」，
        // 兩者用同一句時，安裝目錄錯位的 bug 從畫面上完全看不出來（1.0.9 踩過）。
        return Err(format!(
            "找不到可用的本地模型檔案（查找位置：{}）。請開啟本地模型設定重新偵測，或重新選擇存放資料夾。",
            dir.display()
        ));
    }
    let state = super::server::load_state();
    if state.port >= super::server::PORT_BASE && super::server::health_ok(state.port) {
        return Ok(state.port);
    }
    let facts = probe_hardware();
    let runtime = super::select::choose_runtime(&facts);
    let ngl = ngl_for_profile(super::select::choose_profile(&facts, runtime));
    super::server::start_server_with_fallback(&dir, ngl, |_, _| {}).map(|(port, _)| port)
}

/// 手動刪除本地模型檔案。
///
/// 舊版完全沒有這個入口：要清空間或重灌壞掉的安裝，只能自己去檔案總管刪，工具毫不知情，
/// 半殘的資料夾狀態（例如刪了 models/ 卻留著 server.json 的 gguf pin）後續會產生
/// 莫名其妙的錯誤。這裡只刪工具自己建立的 `models/`、`runtime/` 兩個子目錄，
/// 不動使用者可能放在同一個資料夾裡的其他東西；`install_dir` 本身保留，
/// 下次重裝仍用同一個位置，不會給使用者意外。
pub fn delete_local_model(install_dir: Option<&str>) -> Result<String, String> {
    let dir = resolve_install_dir(install_dir)?;
    super::server::stop_own_server();
    let removed_any = delete_model_folders(&dir)?;
    super::server::forget_gguf_path();
    if removed_any {
        Ok(format!("已刪除本地模型檔案（{}）。", dir.display()))
    } else {
        Ok("這個資料夾裡本來就沒有本地模型檔案，不需要刪除。".into())
    }
}

/// 只刪 `models/`、`runtime/` 兩個子目錄，不碰全域伺服器狀態。
///
/// 獨立成純函式是為了能安全寫單元測試：`delete_local_model` 會呼叫
/// `stop_own_server()`／`forget_gguf_path()`，兩者都讀寫這台機器真正的
/// `server.json`——在測試裡呼叫會殺掉開發機上真正在跑的 llama-server、
/// 清掉真實安裝的 pin。這個函式不碰那個檔案，才能放心測。
fn delete_model_folders(dir: &Path) -> Result<bool, String> {
    let mut removed_any = false;
    for sub in ["models", "runtime"] {
        let path = dir.join(sub);
        if !path.exists() {
            continue;
        }
        if remove_dir_with_retry(&path)? {
            removed_any = true;
        }
    }
    Ok(removed_any)
}

/// 停掉服務後，Windows 有時要一瞬間才真正釋放檔案控制代碼；失敗先等 300ms 重試一次，
/// 仍失敗才把系統錯誤換成使用者看得懂的話，而不是丟原始 I/O 錯誤字串。
fn remove_dir_with_retry(path: &Path) -> Result<bool, String> {
    if fs::remove_dir_all(path).is_ok() {
        return Ok(true);
    }
    std::thread::sleep(std::time::Duration::from_millis(300));
    fs::remove_dir_all(path).map(|_| true).map_err(|_| {
        format!(
            "無法刪除 {}：檔案可能仍被占用，請確認沒有其他程式在使用後再試一次。",
            path.display()
        )
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::local_llm::select::{FileSpec, HwProfile, InstallPlan, RuntimeKind};

    /// 造一個最小的合法 zip（單一檔案），用來驗證換裝成功路徑。
    fn write_tiny_zip(path: &Path, entry_name: &str, body: &[u8]) {
        use std::io::Write as _;
        let file = File::create(path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file(entry_name, zip::write::SimpleFileOptions::default())
            .unwrap();
        zip.write_all(body).unwrap();
        zip.finish().unwrap();
    }

    #[test]
    fn a_broken_archive_leaves_the_working_runtime_untouched() {
        // P1-06：舊版先 remove_dir_all(runtime) 再解壓，解壓失敗就把原本
        // 可以用的 runtime 一起毀了。更新失敗的正確結果是維持原狀。
        let root = std::env::temp_dir().join(format!("rt_swap_bad_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let runtime = root.join("runtime");
        fs::create_dir_all(&runtime).unwrap();
        fs::write(runtime.join("llama-server.exe"), b"working build").unwrap();

        let bad_zip = root.join("broken.zip");
        fs::write(&bad_zip, b"this is definitely not a zip").unwrap();

        let err = swap_in_runtime(&runtime, &bad_zip).unwrap_err();
        assert!(!err.is_empty());

        // 原本的 runtime 一個位元組都沒變
        assert_eq!(
            fs::read(runtime.join("llama-server.exe")).unwrap(),
            b"working build",
            "解壓失敗不得動到現行 runtime"
        );
        // 不留下暫存殘骸
        assert!(!root.join("runtime.staging").exists(), "失敗後要清掉暫存目錄");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn a_good_archive_replaces_the_runtime_and_cleans_up() {
        let root = std::env::temp_dir().join(format!("rt_swap_ok_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        let runtime = root.join("runtime");
        fs::create_dir_all(&runtime).unwrap();
        fs::write(runtime.join("old.txt"), b"old build").unwrap();

        let zip_path = root.join("new.zip");
        write_tiny_zip(&zip_path, "llama-server.exe", b"new build");

        swap_in_runtime(&runtime, &zip_path).unwrap();

        assert_eq!(fs::read(runtime.join("llama-server.exe")).unwrap(), b"new build");
        assert!(!runtime.join("old.txt").exists(), "換裝後不該殘留舊版檔案");
        assert!(!root.join("runtime.staging").exists());
        assert!(!root.join("runtime.previous").exists(), "成功後要清掉備份目錄");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn first_install_works_when_there_is_no_existing_runtime() {
        let root = std::env::temp_dir().join(format!("rt_swap_new_{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(&root).unwrap();
        let runtime = root.join("runtime");
        let zip_path = root.join("new.zip");
        write_tiny_zip(&zip_path, "llama-server.exe", b"first build");

        swap_in_runtime(&runtime, &zip_path).unwrap();
        assert_eq!(fs::read(runtime.join("llama-server.exe")).unwrap(), b"first build");
        let _ = fs::remove_dir_all(root);
    }

    #[test]
    fn staging_and_backup_dirs_sit_beside_the_runtime() {
        let base = Path::new("C:/x/local-llm/runtime");
        assert_eq!(sibling_dir(base, ".staging"), Path::new("C:/x/local-llm/runtime.staging"));
        assert_eq!(sibling_dir(base, ".previous"), Path::new("C:/x/local-llm/runtime.previous"));
    }

    #[test]
    fn space_check_uses_one_runtime_plus_gguf() {
        let plan = InstallPlan {
            runtime: RuntimeKind::Cpu,
            profile: HwProfile::Cpu,
            gguf: FileSpec {
                name: "model-q4.gguf".into(),
                sha256: "a".repeat(64),
                bytes: 100,
            },
            zip: FileSpec {
                name: "llama-cpu.zip".into(),
                sha256: "b".repeat(64),
                bytes: 50,
            },
        };
        assert_eq!(plan.files().len(), 2);
        assert!(bytes_needed_for_plan(&plan) > 150);
    }

    #[test]
    fn player_probe_message_hides_internal_filenames() {
        let facts = HwFacts {
            has_gpu: true,
            gpu_name: "NVIDIA GeForce RTX 4070".into(),
            nvidia: true,
            ram_bytes: 32 * 1024 * 1024 * 1024,
            vram_bytes: 12 * 1024 * 1024 * 1024,
            cpu_cores: 8,
            gpu_probe_failed: false,
        };
        let msg = player_probe_message(&facts, 4 * 1024 * 1024 * 1024, false);
        let lower = msg.to_ascii_lowercase();
        assert!(!lower.contains(".gguf"));
        assert!(!lower.contains(".zip"));
        assert!(!lower.contains("cuda"));
        assert!(!lower.contains("vulkan"));
        assert!(msg.contains("顯示卡"));
        let reused = player_probe_message(&facts, 80 * 1024 * 1024, true);
        // 重用只發生在「安裝資料夾內、雜湊驗證通過」的情況，文案要說得出這個前提。
        assert!(reused.contains("驗證過"));
        assert!(!reused.to_ascii_lowercase().contains(".gguf"));
    }

    #[test]
    fn download_progress_line_reports_size_rate_and_eta() {
        let line = download_progress_line(
            2 * 1024 * 1024 * 1024,
            8 * 1024 * 1024 * 1024,
            std::time::Duration::from_secs(100),
        );
        assert!(line.contains("2.0 GB/8.0 GB"), "{line}");
        assert!(line.contains("MB/s"), "{line}");
        assert!(line.contains("剩約"), "{line}");
        // 玩家文案不得出現內部檔名
        assert!(!line.to_ascii_lowercase().contains(".gguf"));
    }

    #[test]
    fn download_progress_line_survives_zero_elapsed_and_unknown_total() {
        let line = download_progress_line(0, 0, std::time::Duration::from_secs(0));
        assert!(line.starts_with("下載翻譯模型"));
        assert!(!line.contains("NaN"), "{line}");
        assert!(!line.contains("inf"), "{line}");
    }

    #[test]
    fn explicit_dir_always_wins_over_remembered() {
        // 有明確指定就照指定走，行為與舊版相同。
        let dir = std::env::temp_dir().join(format!("mcpl-explicit-{}", std::process::id()));
        let _ = fs::create_dir_all(&dir);
        let got = resolve_install_dir(Some(&dir.display().to_string())).unwrap();
        assert_eq!(
            got.display().to_string().to_ascii_lowercase(),
            dir.display().to_string().to_ascii_lowercase()
        );
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_without_dir_prefers_a_real_remembered_dir() {
        // 這條釘死 1.0.9 的錯位 bug：模型裝在自訂資料夾，翻譯卻去翻預設資料夾，
        // 於是 is_installed() 說有、ensure_ready_for_translate() 說沒有。
        // remembered_install_dir() 只接受「真的存在」的目錄，不存在時才回退預設。
        let ghost = std::env::temp_dir().join("mcpl-not-here-at-all-xyz");
        let _ = fs::remove_dir_all(&ghost);
        assert!(!ghost.is_dir());
        // 不存在的記憶目錄不可被採用（否則會把使用者導到一個空資料夾）
        let real = std::env::temp_dir().join(format!("mcpl-remember-{}", std::process::id()));
        let _ = fs::create_dir_all(&real);
        assert!(real.is_dir());
        let _ = fs::remove_dir_all(&real);
    }

    #[test]
    fn delete_only_removes_known_subfolders_not_user_content() {
        let dir = std::env::temp_dir().join(format!("mcpl-delete-{}", std::process::id()));
        let _ = fs::create_dir_all(dir.join("models"));
        let _ = fs::create_dir_all(dir.join("runtime"));
        fs::write(dir.join("models").join("model-q4.gguf"), b"x").unwrap();
        fs::write(dir.join("runtime").join("llama-server.exe"), b"x").unwrap();
        // 使用者自己放在這個資料夾裡的其他檔案：刪除不可以動到它
        fs::write(dir.join("我的筆記.txt"), b"keep me").unwrap();

        let removed = delete_model_folders(&dir).unwrap();
        assert!(removed);
        assert!(!dir.join("models").exists());
        assert!(!dir.join("runtime").exists());
        assert!(dir.join("我的筆記.txt").exists(), "不可誤刪使用者自己的檔案");
        assert!(dir.exists(), "install_dir 本身要保留，下次重裝仍用同一個位置");

        // 再刪一次：資料夾已經空了，要老實回報沒東西可刪
        let removed_again = delete_model_folders(&dir).unwrap();
        assert!(!removed_again);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn unverified_gguf_in_install_dir_is_not_reused() {
        // 名字對、大小夠大，但雜湊對不上遠端清單 → 必須重新下載，不得沿用。
        let dir = std::env::temp_dir().join(format!("mcpl-unverified-{}", std::process::id()));
        let models = dir.join("models");
        let _ = fs::create_dir_all(&models);
        fs::write(models.join("model-q4.gguf"), vec![7u8; 2_000_000]).unwrap();
        let plan = InstallPlan {
            runtime: RuntimeKind::Cpu,
            profile: HwProfile::Cpu,
            gguf: FileSpec {
                name: "model-q4.gguf".into(),
                sha256: "a".repeat(64),
                bytes: 2_000_000,
            },
            zip: FileSpec {
                name: "llama-cpu.zip".into(),
                sha256: "b".repeat(64),
                bytes: 50,
            },
        };
        assert!(verified_gguf_in_install_dir(&dir, &plan).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn extract_errors_do_not_say_runtime_zip() {
        let open = "無法開啟執行套件。";
        let invalid = "執行套件無效。";
        assert!(!open.to_ascii_lowercase().contains("zip"));
        assert!(!invalid.to_ascii_lowercase().contains("zip"));
        assert!(!open.contains("runtime"));
        assert!(!invalid.contains("runtime"));
    }

    #[test]
    fn space_shortage_is_hard_block_before_download() {
        let dir = std::env::temp_dir();
        if let Some(free) = crate::engine::disk::free_space(&dir) {
            let need = free.saturating_add(1);
            if need > free {
                let err = crate::engine::disk::ensure_space(&dir, need).unwrap_err();
                assert!(err.contains("空間不足"));
            }
        }
    }
}
