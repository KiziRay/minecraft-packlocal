//! 本機硬體偵測：RAM／CPU（sysinfo）＋選定一張用來跑翻譯的顯示卡。
//! 名稱與顯示記憶體必須來自同一張卡；不可寫死特定型號。

use super::select::HwFacts;
use crate::engine::win_process::hidden_command;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GpuCandidate {
    pub name: String,
    pub vram_bytes: u64,
    pub nvidia: bool,
    pub vram_reliable: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum GpuRank {
    Igpu = 1,
    Other = 2,
    IntelArc = 3,
    AmdDiscrete = 4,
    Nvidia = 5,
}

pub fn probe_hardware() -> HwFacts {
    let mut facts = HwFacts::default();
    fill_sysinfo(&mut facts);
    let nvidia_out = run_nvidia_smi();
    let wmi_out = run_wmi_gpus();
    let mut cands = parse_nvidia_smi(&nvidia_out.clone().unwrap_or_default());
    merge_unique(&mut cands, parse_wmi_gpus(&wmi_out.clone().unwrap_or_default()));
    // AdapterRAM 是 32-bit 欄位，大容量卡一律截斷；登錄檔的 qwMemorySize 才是準的。
    apply_registry_vram(&mut cands, &parse_registry_vram(&run_registry_vram().unwrap_or_default()));

    // 「查不到」不等於「沒有」。
    //
    // GPU 偵測靠現場執行 PowerShell（WMI）；防毒攔截、系統忙碌、冷啟動都可能讓它
    // 失敗。舊版失敗就回空清單 → has_gpu = false → choose_profile 直接回 Cpu →
    // ngl = 0，於是使用者那張 20 GB 的 RX 7900 XT 一層都沒用到，整包翻譯跑了
    // 快三小時。這裡把「兩個來源都沒回應」單獨標記出來，讓上層可以改用快取或
    // 已安裝的執行套件來判斷，而不是直接當成沒有顯示卡。
    let probe_failed = cands.is_empty() && wmi_out.is_none() && nvidia_out.is_none();
    apply_picked_gpu(&mut facts, pick_primary_gpu(&cands));
    facts.gpu_probe_failed = probe_failed;
    if facts.has_gpu {
        save_hw_cache(&facts);
    } else if probe_failed {
        // 沿用上次成功的偵測結果——顯示卡不會自己消失
        if let Some(cached) = load_hw_cache() {
            facts.gpu_name = cached.gpu_name;
            facts.vram_bytes = cached.vram_bytes;
            facts.nvidia = cached.nvidia;
            facts.has_gpu = cached.has_gpu;
        }
    }
    facts
}

/// 上次成功偵測到的顯示卡。偵測失敗時沿用，避免一次失敗就退回 CPU。
#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
struct HwCache {
    gpu_name: String,
    vram_bytes: u64,
    nvidia: bool,
    has_gpu: bool,
}

fn hw_cache_path() -> std::path::PathBuf {
    crate::engine::paths::resolve_file(std::path::Path::new("hardware_cache.json"))
}

fn save_hw_cache(facts: &HwFacts) {
    let cache = HwCache {
        gpu_name: facts.gpu_name.clone(),
        vram_bytes: facts.vram_bytes,
        nvidia: facts.nvidia,
        has_gpu: facts.has_gpu,
    };
    let path = hw_cache_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(text) = serde_json::to_string_pretty(&cache) {
        let _ = std::fs::write(path, text);
    }
}

fn load_hw_cache() -> Option<HwCache> {
    let text = std::fs::read_to_string(hw_cache_path()).ok()?;
    let cache: HwCache = serde_json::from_str(&text).ok()?;
    cache.has_gpu.then_some(cache)
}

fn fill_sysinfo(facts: &mut HwFacts) {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    sys.refresh_cpu_all();
    facts.ram_bytes = sys.total_memory();
    facts.cpu_cores = sys.cpus().len() as u32;
}

fn apply_picked_gpu(facts: &mut HwFacts, picked: Option<GpuCandidate>) {
    let Some(gpu) = picked else {
        facts.gpu_name.clear();
        facts.vram_bytes = 0;
        facts.nvidia = false;
        facts.has_gpu = false;
        return;
    };
    facts.gpu_name = gpu.name;
    facts.vram_bytes = gpu.vram_bytes;
    facts.nvidia = gpu.nvidia;
    facts.has_gpu = true;
}

fn run_nvidia_smi() -> Option<String> {
    let out = hidden_command("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

fn run_wmi_gpus() -> Option<String> {
    let out = hidden_command("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            "Get-CimInstance Win32_VideoController | ForEach-Object { $_.Name + '|' + $_.AdapterRAM + '|' + $_.PNPDeviceID }",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn parse_nvidia_smi(stdout: &str) -> Vec<GpuCandidate> {
    let mut out = Vec::new();
    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() || is_virtual_adapter(line) {
            continue;
        }
        let mut parts = line.split(',');
        let name = parts.next().unwrap_or("").trim();
        if name.is_empty() {
            continue;
        }
        let vram_mib: u64 = parts.next().unwrap_or("0").trim().parse().unwrap_or(0);
        out.push(GpuCandidate {
            name: name.to_string(),
            vram_bytes: vram_mib.saturating_mul(1024 * 1024),
            nvidia: true,
            vram_reliable: vram_mib > 0,
        });
    }
    out
}

pub fn parse_wmi_gpus(text: &str) -> Vec<GpuCandidate> {
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(3, '|');
        let name = parts.next().unwrap_or("").trim();
        let ram_raw: u64 = parts.next().unwrap_or("0").trim().parse().unwrap_or(0);
        let pnp = parts.next().unwrap_or("").to_ascii_uppercase();
        if name.is_empty() || is_virtual_adapter(name) {
            continue;
        }
        let nvidia = name.to_ascii_lowercase().contains("nvidia") || pnp.contains("VEN_10DE");
        let vram_bytes = sanitize_adapter_ram(ram_raw);
        out.push(GpuCandidate {
            name: name.to_string(),
            vram_bytes,
            nvidia,
            vram_reliable: false,
        });
    }
    out
}

/// AdapterRAM 只在「明顯小於 4 GiB」時可信。
///
/// 這個欄位是 32-bit，大容量卡一律被截斷，而且**不同驅動截在不同值**：
/// NVIDIA 常見 `0xFFFFFFFF`（4294967295），AMD 實測是 `0xFFF00000`（4293918720，4095 MiB）。
/// 舊版只擋 `>= u32::MAX`，於是 RX 7900 XT（實際 20 GB）從下面鑽過去，被當成 4.0 GB——
/// 顯示錯只是表徵，真正的傷害是 `choose_profile` 因此降到 Low，ngl 從 99 掉到 20，
/// 20 GB 的卡只有 20 層跑在 GPU 上。
///
/// 因此門檻改成「接近 4 GiB 就不可信」：真的只有 4 GB 的卡損失有限（會走登錄檔或保守設定），
/// 誤信一個假的 4 GB 才是會讓高階卡整台變慢的那種錯。
const ADAPTER_RAM_TRUST_CEILING: u64 = 4 * 1024 * 1024 * 1024 - 16 * 1024 * 1024;

fn sanitize_adapter_ram(raw: u64) -> u64 {
    if raw == 0 || raw >= ADAPTER_RAM_TRUST_CEILING {
        return 0;
    }
    raw
}

/// 顯示卡類別的登錄檔位置；`HardwareInformation.qwMemorySize` 是 QWORD，不受 32-bit 上限影響。
fn run_registry_vram() -> Option<String> {
    let script = "Get-ItemProperty -Path 'HKLM:\\SYSTEM\\CurrentControlSet\\Control\\Class\\{4d36e968-e325-11ce-bfc1-08002be10318}\\*' \
                  -ErrorAction SilentlyContinue | Where-Object { $_.DriverDesc } | \
                  ForEach-Object { $_.DriverDesc + '|' + $_.'HardwareInformation.qwMemorySize' }";
    let out = hidden_command("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            script,
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 解析 `DriverDesc|qwMemorySize`。
///
/// 同一張卡常有多筆（不同 driver instance），其中一筆的值可能是空的——實測 RX 7900 XT
/// 就是兩筆、一筆空值。同名取最大值，空值略過。
pub fn parse_registry_vram(text: &str) -> Vec<(String, u64)> {
    let mut out: Vec<(String, u64)> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let mut parts = line.splitn(2, '|');
        let name = parts.next().unwrap_or("").trim();
        let bytes: u64 = parts.next().unwrap_or("").trim().parse().unwrap_or(0);
        if name.is_empty() || bytes == 0 || is_virtual_adapter(name) {
            continue;
        }
        match out.iter_mut().find(|(n, _)| n.eq_ignore_ascii_case(name)) {
            Some((_, existing)) if *existing >= bytes => {}
            Some((_, existing)) => *existing = bytes,
            None => out.push((name.to_string(), bytes)),
        }
    }
    out
}

/// 把登錄檔讀到的 VRAM 套到對應的候選卡上。
///
/// 只在候選卡自己沒有可信 VRAM 時才覆蓋——`nvidia-smi` 的數字比登錄檔更貼近實際可用量，
/// 不要拿登錄檔去蓋掉它。
pub fn apply_registry_vram(cands: &mut [GpuCandidate], registry: &[(String, u64)]) {
    for gpu in cands.iter_mut() {
        if gpu.vram_reliable && gpu.vram_bytes > 0 {
            continue;
        }
        if let Some((_, bytes)) = registry.iter().find(|(name, _)| same_adapter(name, &gpu.name)) {
            gpu.vram_bytes = *bytes;
            gpu.vram_reliable = true;
        }
    }
}

fn is_virtual_adapter(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "microsoft basic display",
        "basic display adapter",
        "remote display",
        "remote desktop",
        "vmware",
        "virtualbox",
        "citrix",
        "parsec",
        "teamviewer",
        "hyper-v",
        "microsoft hyper-v",
    ]
    .iter()
    .any(|n| lower.contains(n))
}

fn merge_unique(dst: &mut Vec<GpuCandidate>, extra: Vec<GpuCandidate>) {
    for gpu in extra {
        let already = dst.iter().any(|e| same_adapter(&e.name, &gpu.name) || (e.nvidia && gpu.nvidia));
        if already {
            continue;
        }
        dst.push(gpu);
    }
}

fn same_adapter(a: &str, b: &str) -> bool {
    let aa = a.to_ascii_lowercase();
    let bb = b.to_ascii_lowercase();
    aa == bb || aa.contains(&bb) || bb.contains(&aa)
}

pub fn pick_primary_gpu(cands: &[GpuCandidate]) -> Option<GpuCandidate> {
    cands
        .iter()
        .filter(|c| !c.name.trim().is_empty())
        .max_by(|a, b| {
            rank_gpu(a)
                .cmp(&rank_gpu(b))
                .then_with(|| a.vram_bytes.cmp(&b.vram_bytes))
        })
        .cloned()
}

fn rank_gpu(gpu: &GpuCandidate) -> GpuRank {
    let lower = gpu.name.to_ascii_lowercase();
    if gpu.nvidia || lower.contains("nvidia") {
        return GpuRank::Nvidia;
    }
    if lower.contains("arc") {
        return GpuRank::IntelArc;
    }
    if lower.contains("amd") || lower.contains("radeon") {
        if is_amd_igpu(&lower) {
            return GpuRank::Igpu;
        }
        return GpuRank::AmdDiscrete;
    }
    if is_intel_igpu(&lower) {
        return GpuRank::Igpu;
    }
    if gpu.vram_bytes > 512 * 1024 * 1024 {
        return GpuRank::Other;
    }
    GpuRank::Igpu
}

fn is_amd_igpu(lower: &str) -> bool {
    (lower.contains("radeon graphics") || lower.contains("radeon(tm) graphics"))
        && !lower.contains("rx")
        && !lower.contains("pro")
        && !lower.contains("vega")
}

fn is_intel_igpu(lower: &str) -> bool {
    lower.contains("uhd")
        || lower.contains("iris")
        || lower.contains("hd graphics")
        || (lower.contains("intel") && !lower.contains("arc"))
}

#[cfg(test)]
mod tests {
    use super::{parse_nvidia_smi, parse_registry_vram, parse_wmi_gpus, pick_primary_gpu, GpuCandidate};
    use super::super::select::{choose_runtime, HwFacts, RuntimeKind};

    #[test]
    fn probe_facts_never_require_rx7900() {
        let facts = HwFacts {
            gpu_name: "Anything GPU".into(),
            has_gpu: true,
            nvidia: false,
            ..HwFacts::default()
        };
        assert_eq!(choose_runtime(&facts), RuntimeKind::Vulkan);
        assert!(!facts.gpu_name.to_ascii_lowercase().contains("7900"));
    }

    #[test]
    fn nvidia_smi_reads_all_rows_and_picks_higher_vram() {
        let rows = parse_nvidia_smi(
            "NVIDIA GeForce RTX 3060, 8192\nNVIDIA GeForce RTX 4070, 12282\n",
        );
        let picked = pick_primary_gpu(&rows).unwrap();
        assert_eq!(picked.name, "NVIDIA GeForce RTX 4070");
        assert_eq!(picked.vram_bytes, 12282 * 1024 * 1024);
    }

    #[test]
    fn laptop_igpu_plus_dgpu_uses_nvidia_name_and_vram() {
        let mut cands = parse_nvidia_smi("NVIDIA GeForce RTX 4060 Laptop GPU, 8188\n");
        cands.extend(parse_wmi_gpus(
            "Intel(R) UHD Graphics|1073741824|PCI\\VEN_8086\nNVIDIA GeForce RTX 4060 Laptop GPU|4294967295|PCI\\VEN_10DE\n",
        ));
        let picked = pick_primary_gpu(&cands).unwrap();
        assert!(picked.name.contains("RTX 4060"));
        assert!(!picked.name.to_ascii_lowercase().contains("uhd"));
        assert_eq!(picked.vram_bytes, 8188 * 1024 * 1024);
        assert!(picked.nvidia);
    }

    #[test]
    fn skips_basic_display_adapter() {
        let cands = parse_wmi_gpus("Microsoft Basic Display Adapter|0|ROOT\\DISPLAY\n");
        assert!(pick_primary_gpu(&cands).is_none());
    }

    #[test]
    fn amd_truncated_adapter_ram_is_not_trusted() {
        // 實機值：RX 7900 XT 的 AdapterRAM 回 0xFFF00000（4095 MiB），實際是 20 GB。
        // 舊版只擋 >= u32::MAX，這個值從下面鑽過去被當成 4.0 GB。
        let cands = parse_wmi_gpus("AMD Radeon RX 7900 XT|4293918720|PCI\\VEN_1002\n");
        assert_eq!(cands[0].vram_bytes, 0, "截斷值必須視為不可信");
        let nv = parse_wmi_gpus("NVIDIA GeForce RTX 4090|4294967295|PCI\\VEN_10DE\n");
        assert_eq!(nv[0].vram_bytes, 0);
        // 真的小於 4 GiB 的卡仍要採信
        let small = parse_wmi_gpus("Some Old GPU|2147483648|PCI\\VEN_1002\n");
        assert_eq!(small[0].vram_bytes, 2147483648);
    }

    #[test]
    fn registry_vram_fills_in_truncated_adapter_ram() {
        let mut cands = parse_wmi_gpus(
            "AMD Radeon(TM) Graphics|33554432|PCI\\VEN_1002\nAMD Radeon RX 7900 XT|4293918720|PCI\\VEN_1002\n",
        );
        // 實機輸出：同名兩筆，其中一筆空值
        let registry = parse_registry_vram(
            "AMD Radeon(TM) Graphics|33554432\nAMD Radeon RX 7900 XT|21458059264\nAMD Radeon RX 7900 XT|\n",
        );
        assert_eq!(registry.len(), 2, "同名重複要合併、空值要略過");
        super::apply_registry_vram(&mut cands, &registry);
        let picked = pick_primary_gpu(&cands).unwrap();
        assert!(picked.name.contains("RX 7900 XT"));
        assert_eq!(picked.vram_bytes, 21458059264);
        assert!(picked.vram_reliable);
    }

    #[test]
    fn registry_does_not_override_nvidia_smi() {
        // nvidia-smi 的數字比登錄檔更貼近實際可用量，不可被蓋掉。
        let mut cands = parse_nvidia_smi("NVIDIA GeForce RTX 4070, 12282\n");
        let registry = parse_registry_vram("NVIDIA GeForce RTX 4070|999999999\n");
        super::apply_registry_vram(&mut cands, &registry);
        assert_eq!(cands[0].vram_bytes, 12282 * 1024 * 1024);
    }

    #[test]
    fn rx7900xt_with_32gb_ram_reaches_high_profile() {
        // 這條是「顯示錯」與「翻譯慢」的交會點：VRAM 誤判成 4 GB 時 profile 掉到 Low，
        // ngl 從 99 變 20。修好偵測之後必須回到 High。
        use super::super::select::{choose_profile, choose_runtime, HwProfile};
        let facts = HwFacts {
            gpu_name: "AMD Radeon RX 7900 XT".into(),
            has_gpu: true,
            nvidia: false,
            vram_bytes: 21458059264,
            ram_bytes: 32 * 1024 * 1024 * 1024,
            cpu_cores: 16,
            gpu_probe_failed: false,
        };
        let runtime = choose_runtime(&facts);
        assert_eq!(runtime, RuntimeKind::Vulkan);
        assert_eq!(choose_profile(&facts, runtime), HwProfile::High);
    }

    #[test]
    fn amd_only_keeps_that_card_vram() {
        let cands = parse_wmi_gpus("AMD Radeon RX 7800 XT|12884901888|PCI\\VEN_1002\n");
        let picked = pick_primary_gpu(&cands).unwrap();
        assert!(picked.name.contains("RX 7800"));
        assert!(!picked.nvidia);
    }

    #[test]
    fn does_not_mix_igpu_name_with_dgpu_vram() {
        let cands = vec![
            GpuCandidate {
                name: "Intel(R) Iris Xe Graphics".into(),
                vram_bytes: 16 * 1024 * 1024 * 1024,
                nvidia: false,
                vram_reliable: false,
            },
            GpuCandidate {
                name: "NVIDIA GeForce RTX 4070".into(),
                vram_bytes: 12 * 1024 * 1024 * 1024,
                nvidia: true,
                vram_reliable: true,
            },
        ];
        let picked = pick_primary_gpu(&cands).unwrap();
        assert_eq!(picked.name, "NVIDIA GeForce RTX 4070");
        assert_eq!(picked.vram_bytes, 12 * 1024 * 1024 * 1024);
    }
}
