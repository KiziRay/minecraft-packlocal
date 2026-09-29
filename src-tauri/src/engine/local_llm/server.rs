//! llama-server：只殺自己的 PID；port 從 18765 起跳。

use serde::{Deserialize, Serialize};
use std::fs;
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::time::{Duration, Instant};

pub const PORT_BASE: u16 = 18765;
pub const PORT_MAX: u16 = 18899;

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ServerState {
    pub pid: u32,
    pub port: u16,
    pub install_dir: String,
    #[serde(default)]
    pub gguf_path: String,
    /// B4：啟動時實際放進顯示卡的層數（0＝純 CPU）。用來估每批該等多久。
    #[serde(default)]
    pub ngl: u32,
    /// B4：啟動時的上下文大小（`-c`），寫日誌與估算用。
    #[serde(default)]
    pub ctx: u32,
    /// B4：啟動時的同時處理數（`--parallel`）；送出端的並行不超過它。0＝舊狀態檔／不知道。
    #[serde(default)]
    pub slots: u32,
    /// B4 F7：模型總層數（gguf `block_count`）；0＝讀不到（速度假設改用保守的 CPU 值）。
    #[serde(default)]
    pub layers: u32,
}

static CHILD: Mutex<Option<Child>> = Mutex::new(None);
/// B4：最近一次啟動的決策說明（上下文、同時處理數、顯示卡層數與理由），給翻譯日誌取用。
static LAST_START_NOTE: Mutex<Option<String>> = Mutex::new(None);

/// B4：我們自己啟動的本地模型程式現在的狀態。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OwnServerLiveness {
    /// 還在跑
    Running,
    /// 已經結束（附結束代碼）
    Exited(String),
    /// 不是這個工具這次啟動的（例如上一次開的），無從判斷
    Unknown,
}

/// B4：程式消失要靠「看程序」偵測，不能只等逾時——逾時要等好幾分鐘，
/// 而程式當掉（多半是記憶體不足）當下就能知道。
pub fn own_server_liveness() -> OwnServerLiveness {
    let mut guard = CHILD.lock().unwrap_or_else(|e| e.into_inner());
    let Some(child) = guard.as_mut() else {
        return OwnServerLiveness::Unknown;
    };
    match child.try_wait() {
        Ok(None) => OwnServerLiveness::Running,
        Ok(Some(status)) => OwnServerLiveness::Exited(match status.code() {
            Some(code) => format!("結束代碼 {code:#x}"),
            None => "被系統結束".to_string(),
        }),
        Err(_) => OwnServerLiveness::Unknown,
    }
}

/// B4：取走最近一次啟動的決策說明（取一次就清掉，避免每批重複寫日誌）。
pub fn take_start_note() -> Option<String> {
    LAST_START_NOTE.lock().unwrap_or_else(|e| e.into_inner()).take()
}

/// 狀態檔位置：三代預設值疊在一起——本地模型第一版用 `%LOCALAPPDATA%`，
/// 第九輪統一改 `%APPDATA%`（跟其餘十個子系統一致），第十輪再統一改成可攜式根
/// （執行檔旁，見 `engine::paths`，使用者要求不擅自佔用系統槽）。
///
/// 既有安裝的 server.json 留在任一代舊路徑時，必須先讀得到它——否則 `is_installed()`
/// 會看到空狀態誤判「尚未安裝」，明明 7GB 模型好好待在磁碟上（見 install.rs 的
/// `resolve_install_dir` 註解，這是同一種 bug 的另一個發生點）。只統一「新安裝」的
/// 預設位置，不強制搬遷既有安裝——找到最新一代裡「有東西」的那個就繼續用它
/// （讀＋寫都是），直到使用者移除該資料夾重新安裝，才會落到最新的可攜式根。
fn state_path() -> PathBuf {
    let portable_root = super::super::paths::portable_root();
    let appdata_root = super::super::paths::legacy_roaming_root();
    let localappdata_root = super::super::paths::legacy_local_root();
    resolve_state_path(&portable_root, &appdata_root, &localappdata_root)
}

/// 純函式版本：給定三個候選根目錄（新到舊），回傳實際要用的 server.json 路徑。
/// 抽出來是因為讀真實環境（執行檔路徑、環境變數）的版本測試裡沒辦法安全覆寫
/// （cargo test 同行程內平行跑，改全域環境變數會互相干擾)。
fn resolve_state_path(portable_root: &Path, appdata_root: &Path, localappdata_root: &Path) -> PathBuf {
    let candidates = [
        portable_root.join("local-llm").join("server.json"),
        appdata_root.join("modpack-i18n-tool").join("local-llm").join("server.json"),
        localappdata_root.join("modpack-i18n-tool").join("local-llm").join("server.json"),
    ];
    for path in &candidates {
        if path.is_file() {
            return path.clone();
        }
    }
    // 都沒有：全新安裝，落在最新的可攜式根。
    candidates[0].clone()
}

pub fn load_state() -> ServerState {
    fs::read_to_string(state_path())
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_state(state: &ServerState) {
    let path = state_path();
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::write(path, serde_json::to_string_pretty(state).unwrap_or_default());
}

pub fn remember_install_dir(dir: &Path) {
    let mut state = load_state();
    state.install_dir = dir.display().to_string();
    save_state(&state);
}

pub fn remember_gguf_path(path: &Path) {
    let mut state = load_state();
    state.gguf_path = path.display().to_string();
    save_state(&state);
}

/// 刪除本地模型檔案後呼叫：檔案不在了，pin 也該跟著清掉，否則 `find_gguf` 會去讀一個
/// 已經被刪除的路徑。保留 `install_dir`，下次重裝仍用同一個資料夾，不會讓使用者驚訝。
pub fn forget_gguf_path() {
    let mut state = load_state();
    state.gguf_path = String::new();
    save_state(&state);
}

fn path_is_inside(candidate: &Path, root: &Path) -> bool {
    let norm = |p: &Path| p.display().to_string().replace('\\', "/").to_ascii_lowercase();
    let (c, r) = (norm(candidate), norm(root));
    let r = r.trim_end_matches('/').to_string();
    c == r || c.starts_with(&format!("{r}/"))
}

/// 清掉指向安裝資料夾外的 gguf pin。
///
/// 舊版會把「機器上任何一個 .gguf」寫進 server.json，而 `find_gguf` 優先用 pin——
/// 於是就算把來源程式碼修好、重新安裝，既有機器上的 pin 仍然生效。這是遷移用的清除，
/// 每次 probe／install 都跑一次，成本只有一次讀寫。
pub fn forget_foreign_gguf_pin(install_dir: &Path) {
    let state = load_state();
    let pinned = state.gguf_path.trim();
    if pinned.is_empty() {
        return;
    }
    if path_is_inside(Path::new(pinned), install_dir) {
        return;
    }
    // pin 在「記住的安裝目錄」底下也算數：使用者把模型裝在 D:\bot\mo 時，
    // 若這次剛好以預設目錄呼叫，不該把那個完全正常的 pin 當成外來的清掉。
    let remembered = state.install_dir.trim();
    if !remembered.is_empty() && path_is_inside(Path::new(pinned), Path::new(remembered)) {
        return;
    }
    let mut next = state.clone();
    next.gguf_path = String::new();
    save_state(&next);
}

pub(crate) fn is_sidecar_weight(name: &str) -> bool {
    let n = name.to_ascii_lowercase();
    n.contains("mmproj") || n.starts_with("mtp-") || n.contains("-mtp-") || n.contains(".mtp.")
}

pub(crate) fn find_main_gguf(dir: &Path) -> Option<PathBuf> {
    let mut best: Option<(u64, PathBuf)> = None;
    let roots = [dir.to_path_buf(), dir.join("models")];
    for root in roots {
        if !root.is_dir() {
            continue;
        }
        let Ok(walk) = fs::read_dir(&root) else {
            continue;
        };
        for ent in walk.flatten() {
            let p = ent.path();
            if p.extension().and_then(|e| e.to_str()) != Some("gguf") {
                continue;
            }
            let name = match p.file_name().and_then(|n| n.to_str()) {
                Some(n) => n,
                None => continue,
            };
            if is_sidecar_weight(name) {
                continue;
            }
            let Ok(meta) = fs::metadata(&p) else {
                continue;
            };
            if !meta.is_file() || meta.len() < 1_000_000 {
                continue;
            }
            if best.as_ref().map(|(size, _)| meta.len() > *size).unwrap_or(true) {
                best = Some((meta.len(), p));
            }
        }
    }
    best.map(|(_, p)| p)
}

pub fn pick_port() -> Result<u16, String> {
    for port in PORT_BASE..=PORT_MAX {
        if TcpListener::bind(("127.0.0.1", port)).is_ok() {
            return Ok(port);
        }
    }
    Err("找不到可用的本機連接埠（18765–18899）。".into())
}

pub fn chat_base_url(port: u16) -> String {
    format!("http://127.0.0.1:{port}")
}

pub fn health_ok(port: u16) -> bool {
    let client = match reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(3))
        .build()
    {
        Ok(c) => c,
        Err(_) => return false,
    };
    client
        .get(format!("http://127.0.0.1:{port}/health"))
        .send()
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

pub(crate) fn find_llama_exe(install_dir: &Path) -> Result<PathBuf, String> {
    let direct = install_dir.join("llama-server.exe");
    if direct.is_file() {
        return Ok(direct);
    }
    let runtime = install_dir.join("runtime");
    if runtime.is_dir() {
        if let Ok(walk) = fs::read_dir(&runtime) {
            for ent in walk.flatten() {
                let p = ent.path();
                if p.is_file()
                    && p.file_name()
                        .and_then(|n| n.to_str())
                        .map(|n| n.eq_ignore_ascii_case("llama-server.exe"))
                        .unwrap_or(false)
                {
                    return Ok(p);
                }
                if p.is_dir() {
                    let nested = p.join("llama-server.exe");
                    if nested.is_file() {
                        return Ok(nested);
                    }
                }
            }
        }
    }
    Err("找不到執行程式，請先完成本地模型安裝。".into())
}

fn find_gguf(install_dir: &Path) -> Result<PathBuf, String> {
    let state = load_state();
    if !state.gguf_path.trim().is_empty() {
        let pinned = PathBuf::from(state.gguf_path.trim());
        // pin 只在安裝資料夾內才作數；外部絕對路徑一律忽略（見 forget_foreign_gguf_pin）。
        if pinned.is_file() && path_is_inside(&pinned, install_dir) {
            return Ok(pinned);
        }
    }
    if let Some(found) = find_main_gguf(install_dir) {
        return Ok(found);
    }
    Err("找不到翻譯模型，請先完成安裝或改選已有模型的資料夾。".into())
}

pub fn files_ready(install_dir: &Path) -> bool {
    find_llama_exe(install_dir).is_ok() && find_gguf(install_dir).is_ok()
}

pub fn stop_own_server() {
    if let Ok(mut g) = CHILD.lock() {
        if let Some(mut child) = g.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
    let state = load_state();
    if state.pid > 0 {
        #[cfg(windows)]
        {
            let _ = crate::engine::win_process::hidden_command("taskkill")
                .args(["/PID", &state.pid.to_string(), "/F"])
                .output();
        }
    }
    save_state(&ServerState {
        pid: 0,
        port: 0,
        install_dir: state.install_dir,
        gguf_path: state.gguf_path,
        ngl: 0,
        ctx: 0,
        slots: 0,
        layers: 0,
    });
}

/// 這台機器**跑不跑得動**這套執行程式。
///
/// `find_llama_exe` 只確認檔案存在，不代表它能執行：顯示卡驅動太舊、處理器缺指令集、
/// 缺相依 DLL 都會在真正啟動時才爆，而那時候使用者已經下載完 7.38 GB 的模型了。
/// 這裡在解壓完就實跑一次，35 MB 的代價即可判定。
///
/// 失效方向朝「放行」：只要行程起得來、沒有立刻因載入錯誤而死，就算通過。
/// 有些 build 的 `--version` 不回 0，不能拿退出碼當唯一判準。
pub fn runtime_can_run(install_dir: &Path) -> Result<(), String> {
    let exe = find_llama_exe(install_dir)?;
    let out = crate::engine::win_process::hidden_command(&exe)
        .arg("--version")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("無法執行：{e}"))?;
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    // Windows 載入失敗的典型訊息；出現這些就是真的不相容，不是單純退出碼非 0。
    let lower = text.to_ascii_lowercase();
    let load_failed = lower.contains("0xc000007b")
        || lower.contains("is not a valid win32")
        || lower.contains("dll")
            && (lower.contains("not found") || lower.contains("missing") || lower.contains("找不到"));
    if load_failed {
        return Err("執行程式無法載入（缺少相依元件或與這台電腦不相容）".into());
    }
    if !out.status.success() && text.trim().is_empty() {
        return Err("執行程式無法啟動".into());
    }
    Ok(())
}

/// 「就緒」＝真的翻得出東西，而不是 `/health` 回 200。
///
/// 舊版把健康檢查當成安裝成功的判準，於是會發生「安裝顯示成功，第一批翻譯就失敗」：
/// 模型載入了但記憶體不夠、或上下文設定不合，健康端點照樣回 200。
/// 這裡送一句極短的翻譯，要求拿到非空回應才算通過。
pub fn verify_can_translate(port: u16) -> Result<(), String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(90))
        .build()
        .map_err(|_| "無法建立連線".to_string())?;
    let body = serde_json::json!({
        "messages": [
            { "role": "system", "content": "You translate to Traditional Chinese. Reply with the translation only." },
            { "role": "user", "content": "Iron Sword" }
        ],
        // 現場實測過：不關掉思考模式，這個請求連自己的驗證步驟都會失敗——32 個 token
        // 全被推理過程吃光，content 是空字串，跟 deepseek.rs 的翻譯請求踩的是同一個坑
        // （見那邊的完整說明）。這裡也要關，否則「安裝完成」的驗證會誤判成功模型為壞的。
        "chat_template_kwargs": { "enable_thinking": false },
        "max_tokens": 32,
        "temperature": 0.0,
        "stream": false,
    });
    let resp = client
        .post(format!("{}/v1/chat/completions", chat_base_url(port)))
        .header("Authorization", format!("Bearer {LOCAL_LLM_API_KEY}"))
        .json(&body)
        .send()
        .map_err(|_| memory_shortfall_message("本地模型沒有回應"))?;
    if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
        // 這代表啟動時傳的 --api-key 沒有生效——理論上不該發生，但萬一發生要一眼看出
        // 不是「記憶體不足」，避免使用者對著錯誤原因排查。
        return Err(
            "本地模型拒絕了連線金鑰，這是工具內部問題，請用頁尾「回報」告訴我們。".into(),
        );
    }
    if !resp.status().is_success() {
        return Err(memory_shortfall_message("本地模型載入了，但無法產生翻譯"));
    }
    let value: serde_json::Value = resp
        .json()
        .map_err(|_| "本地模型回應格式不正確。".to_string())?;
    let text = value["choices"][0]["message"]["content"]
        .as_str()
        .unwrap_or("")
        .trim()
        .to_string();
    if text.is_empty() {
        return Err(memory_shortfall_message("本地模型可以啟動，但翻不出東西（回應是空的）"));
    }
    Ok(())
}

/// 三個「大概是記憶體不夠」的失敗點統一收斂成同一句結論與同一條替代路徑。
///
/// `preflight()` 只是估算，這裡是實際載入後才發現的落差；兩邊用詞一致，
/// 使用者不會覺得「怎麼每個地方講法都不一樣」。
fn memory_shortfall_message(what_happened: &str) -> String {
    format!(
        "{what_happened}，通常是這台電腦的記憶體不足以跑這個模型。\n\
         目前只提供這一款模型，沒有更小的版本可以換。\n\
         可以關閉其他程式後再試，或改用自訂 API／GPT 翻譯，兩者都不佔用本機記憶體。"
    )
}

/// KV 上下文視窗大小。
///
/// 曾經寫死 4096，但翻譯請求的 `max_tokens` 最高會要到 8192——**輸出上限比整個上下文還大**，
/// 長任務書／書本必定截斷或直接失敗，對使用者呈現為「已安裝但翻譯失敗」。
/// 這裡依系統記憶體選一個安全值；輸出上限那一側另外在 deepseek.rs 夾住。
/// B4：保底值改由 sizing.rs 統一計算（實際啟動用 `sizing::plan_context`）；這支留給不變式測試。
#[allow(dead_code)]
pub fn context_size_for(ram_bytes: u64) -> u32 {
    super::sizing::baseline_context_for(ram_bytes)
}

pub fn start_server(install_dir: &Path, ngl: u32) -> Result<u16, String> {
    // B4：上下文依「每批需要多少 × 同時處理幾個」回推，再用記憶體封頂（見 sizing.rs）。
    // 舊版只看記憶體分三級，開 3 個 slot 時每個請求可能分不到一批所需的量。
    let ram = total_ram_bytes();
    let model_bytes = find_gguf(install_dir)
        .ok()
        .and_then(|p| fs::metadata(p).ok())
        .map(|m| m.len())
        .unwrap_or(0);
    let slots = parallel_slots_for(ram, physical_cores());
    let plan = super::sizing::plan_context(
        ram,
        model_bytes,
        slots,
        crate::engine::deepseek::LOCAL_MAX_BATCH_ITEMS as u32,
        crate::engine::deepseek::LOCAL_LLM_MAX_COMPLETION_TOKENS as u32,
    );
    let note = plan.note(ngl);
    crate::dev_log!("local", "{note}");
    let result = start_server_with_slots(install_dir, ngl, plan.ctx, plan.slots);
    if result.is_ok() {
        *LAST_START_NOTE.lock().unwrap_or_else(|e| e.into_inner()) = Some(note);
    }
    result
}

fn total_ram_bytes() -> u64 {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.total_memory()
}

/// 同時處理幾個翻譯請求。
///
/// 這個工作負載是「大量的短請求」——每個請求之間，CPU 在等前處理／後處理／
/// HTTP 往返時是閒著的。舊版寫死 `--parallel 1`，等於把這些空檔全部浪費掉；
/// 使用者實測一包整合包翻了快三小時。
///
/// 併發數不能亂開：每個 slot 都要自己的 KV cache，記憶體不夠會變慢甚至失敗。
/// 所以依「實體記憶體」與「核心數」取保守的較小值：
/// - 記憶體 < 8 GB：維持 1（開併發只會更糟）
/// - 8～16 GB：2
/// - 16 GB 以上：3
/// 再與「核心數 / 2」取小值，避免核心太少時互相搶 CPU 反而更慢。
pub fn parallel_slots_for(ram_bytes: u64, cores: usize) -> u32 {
    const GB: u64 = 1024 * 1024 * 1024;
    let by_ram = if ram_bytes < 8 * GB {
        1
    } else if ram_bytes < 16 * GB {
        2
    } else {
        3
    };
    let by_cores = (cores / 2).max(1) as u32;
    by_ram.min(by_cores).max(1)
}

fn physical_cores() -> usize {
    std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(4)
}

/// 這台電腦建議的併發數。送出端（`provider_capabilities`）與伺服器端
/// （`--parallel`）必須用同一個值，否則多開的 slot 沒人用、或請求排隊更慢。
pub fn recommended_parallel_slots() -> u32 {
    parallel_slots_for(total_ram_bytes(), physical_cores())
}

/// 我們自己啟動、自己呼叫的本地服務用的固定金鑰。
///
/// llama-server 支援 `--api-key`（`env: LLAMA_API_KEY`）。舊版沒有明確傳這個參數，
/// 於是它會撿到**系統上任何其他工具設定的 `LLAMA_API_KEY` 環境變數**——這台開發機
/// 就因為裝過別的本機 AI 工具而設了 `LLAMA_API_KEY=llamacpp`，MCPL 送的 `Bearer local`
/// 對不上，所有真正的翻譯請求都被拒絕（401），但 `/health` 不驗證金鑰，健康檢查照樣過。
/// 結果就是「安裝顯示成功，一翻譯就失敗」，而且任何裝過 Ollama／LM Studio 之類
/// 工具的玩家電腦都可能中招，不是這台機器獨有。
///
/// 明確傳入 `--api-key` 是 CLI 參數，會蓋過環境變數的預設值，徹底不受環境汙染影響。
pub const LOCAL_LLM_API_KEY: &str = "mcpl-local-llm";

#[allow(dead_code)] // 舊入口（依記憶體自算同時處理數）；B4 起啟動走 start_server → plan_context
pub fn start_server_with_ctx(install_dir: &Path, ngl: u32, ctx: u32) -> Result<u16, String> {
    start_server_with_slots(install_dir, ngl, ctx, parallel_slots_for(total_ram_bytes(), physical_cores()))
}

/// B4：同時處理數由呼叫端決定（記憶體不夠時 sizing 會降成 1），並記進狀態讓送出端照著用。
fn start_server_with_slots(install_dir: &Path, ngl: u32, ctx: u32, slots: u32) -> Result<u16, String> {
    let exe = find_llama_exe(install_dir)?;
    let gguf = find_gguf(install_dir)?;
    let port = pick_port()?;
    let cores = physical_cores();
    let slots = slots.max(1);
    // 留一顆核心給工具本身與系統，避免翻譯期間整台電腦卡住
    let threads = cores.saturating_sub(1).max(1);
    stop_own_server();
    let mut cmd = Command::new(&exe);
    cmd.args([
        "--host",
        "127.0.0.1",
        "--port",
        &port.to_string(),
        "-m",
        &gguf.display().to_string(),
        "-c",
        &ctx.to_string(),
        // 依這台電腦的記憶體與核心數決定同時處理幾個請求。翻譯是「大量短請求」，
        // 序列送會把請求之間的 CPU 空檔全部浪費掉（實測一包要跑近三小時）。
        "--parallel",
        &slots.to_string(),
        "-ngl",
        &ngl.to_string(),
        // 明確指定執行緒數：交給預設值時，llama.cpp 在部分 Windows 機器上會挑到
        // 邏輯核心數（含超執行緒），純 CPU 推論反而互相搶資源。
        "-t",
        &threads.to_string(),
        "--jinja",
        "--api-key",
        LOCAL_LLM_API_KEY,
    ])
    .current_dir(install_dir)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null());
    crate::engine::win_process::hide_console(&mut cmd);
    let child = cmd
        .spawn()
        .map_err(|e| format!("無法啟動本地模型：{e}"))?;
    let pid = child.id();
    // B5b 第二輪審查（啟動中選「仍要關閉」會不會留下孤兒）：quit_app 的 stop_own_server 若跑在 spawn 之前、
    // 這裡的 CHILD 還沒存進去，接著的 app.exit 結束本程式 → 下一行綁上的 Job Object（KILL_ON_JOB_CLOSE）
    // 控制代碼被 OS 收回，llama-server 跟著結束。唯一沒保護的是 spawn() 與綁 Job 之間的極短空檔（推測，未實測）。
    // 工具被工作管理員強制結束、當機等優雅關閉路徑跑不到的情況，讓 OS 自己收掉
    // 這個子程序，不要留下孤兒佔用記憶體與顯示卡（見 kill_child_when_this_process_dies）。
    crate::engine::win_process::kill_child_when_this_process_dies(&child);
    if let Ok(mut g) = CHILD.lock() {
        *g = Some(child);
    }
    let mut next = load_state();
    next.pid = pid;
    next.port = port;
    next.ngl = ngl;
    next.ctx = ctx;
    next.slots = slots;
    next.layers = super::gguf_meta::read_block_count(&gguf).unwrap_or(0);
    crate::dev_log!("local", "模型總層數：{}（0＝讀不到，速度改用 CPU 假設）", next.layers);
    // 新的程式、新的速度：上一輪量到的速度不再準
    super::sizing::reset_speed();
    next.install_dir = install_dir.display().to_string();
    if next.gguf_path.trim().is_empty() {
        if let Ok(found) = find_gguf(install_dir) {
            next.gguf_path = found.display().to_string();
        }
    }
    save_state(&next);
    let deadline = Instant::now() + Duration::from_secs(45);
    while Instant::now() < deadline {
        crate::engine::cancel::check()?;
        if health_ok(port) {
            return Ok(port);
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    Err("本地模型已啟動，但健康檢查逾時。請稍後再試。".into())
}

/// 從 `ngl` 往下降的重試階梯。
///
/// `-ngl 99`（全部層放進顯示記憶體）在 VRAM 不夠時會直接啟動失敗或 OOM。舊版沒有任何
/// 降級，於是「這台電腦裝得起來但跑不起來」。少放幾層只是變慢，總比完全不能用好。
/// 最後一階一定是 0＝純 CPU，那是任何機器都能跑的保底。
pub fn ngl_ladder(start: u32) -> Vec<u32> {
    let mut out: Vec<u32> = [start, 40, 20, 8, 0]
        .into_iter()
        .filter(|v| *v <= start)
        .collect();
    out.dedup();
    if out.last() != Some(&0) {
        out.push(0);
    }
    out
}

/// 依 ngl 階梯逐階嘗試啟動；回傳 (port, 實際用的 ngl)。
pub fn start_server_with_fallback(
    install_dir: &Path,
    ngl: u32,
    mut on_step: impl FnMut(u32, &str),
) -> Result<(u16, u32), String> {
    let ladder = ngl_ladder(ngl);
    let mut last_err = String::new();
    for (i, level) in ladder.iter().copied().enumerate() {
        if i > 0 {
            on_step(
                level,
                "顯示記憶體不足以載入整個模型，改用較保守的設定重試（會慢一些）…",
            );
        }
        match start_server(install_dir, level) {
            Ok(port) => return Ok((port, level)),
            Err(err) => {
                crate::engine::cancel::check()?;
                last_err = err;
            }
        }
    }
    Err(if last_err.is_empty() {
        "本地模型無法啟動。".into()
    } else {
        last_err
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parallel_slots_scale_with_hardware_but_stay_conservative() {
        const GB: u64 = 1024 * 1024 * 1024;
        // 記憶體不足的機器維持序列：開併發只會讓它更慢甚至失敗
        assert_eq!(parallel_slots_for(4 * GB, 8), 1);
        assert_eq!(parallel_slots_for(6 * GB, 16), 1);
        // 一般機器開 2
        assert_eq!(parallel_slots_for(12 * GB, 8), 2);
        // 記憶體充足＋核心多才開到 3
        assert_eq!(parallel_slots_for(32 * GB, 16), 3);
        // 核心太少時不能因為記憶體大就亂開，會互相搶 CPU
        assert_eq!(parallel_slots_for(32 * GB, 4), 2);
        assert_eq!(parallel_slots_for(32 * GB, 2), 1);
        // 任何情況都至少是 1，不能回 0（會讓 llama-server 啟動失敗）
        assert_eq!(parallel_slots_for(0, 0), 1);
    }

    #[test]
    fn memory_shortfall_messages_say_there_is_only_one_model() {
        // item 5：硬體不允許時要老實講「只提供這一款」，不能暗示有更小的版本可以退。
        let msg = memory_shortfall_message("測試情境");
        assert!(msg.contains("只提供這一款模型"));
        assert!(msg.contains("自訂 API"));
        assert!(msg.contains("GPT"));
        assert!(!msg.to_ascii_lowercase().contains("gguf"));
    }

    #[test]
    fn local_llm_api_key_is_fixed_and_non_empty() {
        // 這把 key 必須是固定值、不能是空字串：空字串會讓 secrets.rs 那邊的
        // `!engine.api_key.is_empty()` 判斷失效，Authorization header 就不會送出。
        assert!(!LOCAL_LLM_API_KEY.is_empty());
        assert_eq!(LOCAL_LLM_API_KEY, "mcpl-local-llm");
    }

    #[test]
    fn chat_url_is_loopback_only() {
        assert_eq!(chat_base_url(18765), "http://127.0.0.1:18765");
        assert!(PORT_BASE >= 18765);
    }

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "mcpl-state-path-test-{tag}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn resolve_state_path_prefers_portable_root_when_it_already_has_state() {
        // 第十輪：統一放可攜式根——可攜式根已經有 server.json 時，不用理會另外兩個舊根。
        let root = scratch("portable-exists");
        let portable = root.join("portable");
        let appdata = root.join("appdata");
        let localappdata = root.join("localappdata");
        let portable_file = portable.join("local-llm").join("server.json");
        fs::create_dir_all(portable_file.parent().unwrap()).unwrap();
        fs::write(&portable_file, "{}").unwrap();
        assert_eq!(
            resolve_state_path(&portable, &appdata, &localappdata),
            portable_file
        );
    }

    #[test]
    fn resolve_state_path_falls_back_to_appdata_when_only_appdata_has_state() {
        // 第九輪的既有安裝（server.json 在 %APPDATA%）：可攜式根還沒有東西時要沿用它，
        // 不能因為多了一個更新的預設值就假裝這個使用者沒裝過。
        let root = scratch("appdata-only");
        let portable = root.join("portable");
        let appdata = root.join("appdata");
        let localappdata = root.join("localappdata");
        let appdata_file = appdata
            .join("modpack-i18n-tool")
            .join("local-llm")
            .join("server.json");
        fs::create_dir_all(appdata_file.parent().unwrap()).unwrap();
        fs::write(&appdata_file, "{}").unwrap();
        assert_eq!(
            resolve_state_path(&portable, &appdata, &localappdata),
            appdata_file
        );
    }

    #[test]
    fn resolve_state_path_falls_back_to_localappdata_when_only_localappdata_has_state() {
        // 第八輪以前的既有安裝（server.json 還留在舊的 %LOCALAPPDATA%）：is_installed()
        // 靠這個才能讀到 install_dir，不然明明裝過模型卻被判定「尚未安裝」。
        let root = scratch("localappdata-only");
        let portable = root.join("portable");
        let appdata = root.join("appdata");
        let localappdata = root.join("localappdata");
        let local_file = localappdata
            .join("modpack-i18n-tool")
            .join("local-llm")
            .join("server.json");
        fs::create_dir_all(local_file.parent().unwrap()).unwrap();
        fs::write(&local_file, "{}").unwrap();
        assert_eq!(
            resolve_state_path(&portable, &appdata, &localappdata),
            local_file
        );
    }

    #[test]
    fn resolve_state_path_defaults_to_portable_root_when_none_exist() {
        // 全新安裝：三邊都沒有 server.json，落到可攜式根（不建立檔案，交給 save_state）。
        let root = scratch("neither");
        let portable = root.join("portable");
        let appdata = root.join("appdata");
        let localappdata = root.join("localappdata");
        let expected = portable.join("local-llm").join("server.json");
        assert_eq!(
            resolve_state_path(&portable, &appdata, &localappdata),
            expected
        );
    }

    #[test]
    fn sidecar_weights_are_skipped() {
        assert!(is_sidecar_weight("mmproj-Gemma4-12B-BF16.gguf"));
        assert!(is_sidecar_weight("mtp-gemma-4-12B-it.gguf"));
        assert!(!is_sidecar_weight(
            "Gemma4-12B-QAT-Uncensored-HauhauCS-Balanced-Q4_K_M.gguf"
        ));
    }

    #[test]
    fn context_is_never_smaller_than_the_output_cap() {
        // 上下文必須容得下「輸出上限 + prompt」。輸出那一側夾在 2048（見 deepseek.rs），
        // 所以最小上下文 8192 表示 prompt 還有 6144 可用。
        for ram_gb in [4u64, 8, 16, 32, 64, 128] {
            let ctx = context_size_for(ram_gb * 1024 * 1024 * 1024);
            assert!(ctx >= 8192, "{ram_gb} GB 得到 ctx={ctx}");
            assert!(ctx >= 2048 * 4, "輸出上限只能佔 ctx 的四分之一");
        }
        assert_eq!(context_size_for(32 * 1024 * 1024 * 1024), 16384);
        assert_eq!(context_size_for(8 * 1024 * 1024 * 1024), 8192);
    }

    #[test]
    fn ngl_ladder_descends_and_always_ends_at_cpu() {
        let full = ngl_ladder(99);
        assert_eq!(full.first(), Some(&99), "第一階必須是原本算出來的值");
        assert_eq!(full.last(), Some(&0), "最後一階必須是純 CPU 保底");
        for w in full.windows(2) {
            assert!(w[0] > w[1], "階梯必須遞減：{full:?}");
        }
        // 低階顯卡：不可出現比起點還高的層數
        let low = ngl_ladder(20);
        assert!(low.iter().all(|v| *v <= 20), "{low:?}");
        assert_eq!(low.last(), Some(&0));
        // 本來就是 CPU：只有一階
        assert_eq!(ngl_ladder(0), vec![0]);
    }

    #[test]
    fn foreign_pin_is_rejected_and_cleared() {
        let install = std::env::temp_dir().join(format!("mcpl-pin-{}", std::process::id()));
        let outside = std::env::temp_dir().join(format!("mcpl-outside-{}", std::process::id()));
        let _ = fs::create_dir_all(install.join("models"));
        let _ = fs::create_dir_all(&outside);
        let foreign = outside.join("stranger.gguf");
        fs::write(&foreign, vec![0u8; 2_000_000]).unwrap();
        assert!(!path_is_inside(&foreign, &install));
        assert!(path_is_inside(&install.join("models").join("a.gguf"), &install));
        let _ = fs::remove_dir_all(&install);
        let _ = fs::remove_dir_all(&outside);
    }

    #[test]
    fn find_main_gguf_picks_largest_non_sidecar() {
        let dir = std::env::temp_dir().join(format!("mcpl-gguf-{}", std::process::id()));
        let models = dir.join("models");
        let _ = fs::create_dir_all(&models);
        fs::write(models.join("mmproj-x.gguf"), vec![0u8; 2_000_000]).unwrap();
        fs::write(models.join("tiny-placeholder.gguf"), vec![0u8; 16]).unwrap();
        fs::write(models.join("main-q4.gguf"), vec![0u8; 4_000_000]).unwrap();
        let found = find_main_gguf(&dir).expect("main weight");
        assert_eq!(found.file_name().unwrap(), "main-q4.gguf");
        let _ = fs::remove_dir_all(&dir);
    }
}
