//! 依硬體只選一套 runtime zip + Q4 GGUF。不預抓另外兩套。

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeKind {
    Cuda,
    Vulkan,
    Cpu,
}

impl RuntimeKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cuda => "cuda",
            Self::Vulkan => "vulkan",
            Self::Cpu => "cpu",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HwProfile {
    High,
    Med,
    Low,
    Cpu,
}

impl HwProfile {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::High => "high",
            Self::Med => "med",
            Self::Low => "low",
            Self::Cpu => "cpu",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct HwFacts {
    pub ram_bytes: u64,
    pub cpu_cores: u32,
    pub gpu_name: String,
    pub vram_bytes: u64,
    pub nvidia: bool,
    pub has_gpu: bool,
    /// GPU 偵測本身失敗（WMI／nvidia-smi 都沒回應），不是「確定沒有顯示卡」。
    /// 兩者的處置完全不同：確定沒有 → CPU；查不到 → 不能就這樣退回 CPU。
    pub gpu_probe_failed: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FileSpec {
    pub name: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone)]
pub struct InstallPlan {
    pub runtime: RuntimeKind,
    pub profile: HwProfile,
    pub gguf: FileSpec,
    pub zip: FileSpec,
}

impl InstallPlan {
    pub fn files(&self) -> [&FileSpec; 2] {
        [&self.gguf, &self.zip]
    }

    pub fn total_bytes(&self) -> u64 {
        self.gguf.bytes.saturating_add(self.zip.bytes)
    }
}

/// NVIDIA→CUDA；其他 GPU（AMD／Intel／Vulkan 裝置）→Vulkan；無 GPU→CPU。
pub fn choose_runtime(facts: &HwFacts) -> RuntimeKind {
    if facts.nvidia {
        RuntimeKind::Cuda
    } else if facts.has_gpu {
        RuntimeKind::Vulkan
    } else {
        RuntimeKind::Cpu
    }
}

pub fn choose_profile(facts: &HwFacts, runtime: RuntimeKind) -> HwProfile {
    // 已經裝了 GPU 版執行套件（Vulkan／CUDA），代表安裝當下確實偵測到顯示卡。
    // 這種情況下就算後來某次偵測失敗，也不該把 ngl 降到 0——使用者那張 20 GB
    // 的卡就是這樣被閒置、整包翻譯跑了快三小時。
    if !matches!(runtime, RuntimeKind::Cpu) && facts.gpu_probe_failed {
        return HwProfile::Med;
    }
    if matches!(runtime, RuntimeKind::Cpu) || !facts.has_gpu {
        return HwProfile::Cpu;
    }
    let ram_gb = facts.ram_bytes / (1024 * 1024 * 1024);
    let vram_gb = facts.vram_bytes / (1024 * 1024 * 1024);
    // VRAM 讀不出來（驅動不給、登錄檔沒有）但確實有獨顯時，不要當成弱機。
    // 舊行為會掉到 Low（ngl 20），高階卡因此大半跑在 CPU 上。這裡取中間值 Med（ngl 40）：
    // 對真的弱卡不至於爆顯示記憶體，對強卡也不會慢到離譜；讀得到就一律照實際值判斷。
    if vram_gb == 0 {
        return if ram_gb >= 16 { HwProfile::Med } else { HwProfile::Low };
    }
    if ram_gb >= 32 && vram_gb >= 12 {
        HwProfile::High
    } else if ram_gb >= 16 && vram_gb >= 8 {
        HwProfile::Med
    } else {
        HwProfile::Low
    }
}

#[derive(Debug, Clone, Default)]
pub struct RemoteManifest {
    #[allow(dead_code)]
    pub version: u32,
    #[allow(dead_code)]
    pub model_id: String,
    pub gguf: FileSpec,
    pub cuda: FileSpec,
    pub vulkan: FileSpec,
    pub cpu: FileSpec,
}

pub fn plan_from_manifest(facts: &HwFacts, manifest: &RemoteManifest) -> Result<InstallPlan, String> {
    if manifest.gguf.name.trim().is_empty() {
        return Err("遠端清單缺少 GGUF 檔名。".into());
    }
    let runtime = choose_runtime(facts);
    let zip = match runtime {
        RuntimeKind::Cuda => manifest.cuda.clone(),
        RuntimeKind::Vulkan => manifest.vulkan.clone(),
        RuntimeKind::Cpu => manifest.cpu.clone(),
    };
    if zip.name.trim().is_empty() {
        return Err(format!("遠端清單缺少 {} runtime。", runtime.as_str()));
    }
    Ok(InstallPlan {
        runtime,
        profile: choose_profile(facts, runtime),
        gguf: manifest.gguf.clone(),
        zip,
    })
}

/// 空間：檔案總和 + 512MB 餘裕（解壓）。不足則硬擋、不下載。
pub const SPACE_MARGIN_BYTES: u64 = 512 * 1024 * 1024;

/// 載入模型需要的總記憶體（系統記憶體＋顯示記憶體合計）。
///
/// 模型權重一定要整份放得下，另外留 2 GB 給作業系統、KV 快取與工作緩衝。
/// 用「合計」而不是只看系統記憶體，是因為層放到顯示卡上時 VRAM 也在分擔——
/// 20 GB 顯卡＋16 GB 記憶體的機器跑 7.4 GB 模型完全沒問題。
pub fn min_total_memory_for_model(model_bytes: u64) -> u64 {
    model_bytes.saturating_add(2 * 1024 * 1024 * 1024)
}

fn gb_text(bytes: u64) -> String {
    format!("{:.1} GB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

/// 前置檢查的結論。`Ok(())` 才值得開始下載。
///
/// 回傳的是**具體缺什麼**而不是布林值：舊版只檢查磁碟，記憶體不足要等到模型下載完、
/// 服務啟動時才爆，那時使用者已經付出整包下載的代價。
///
/// 失效方向朝放行：偵測不到記憶體（0）時不擋人。
pub fn preflight(facts: &HwFacts, model_bytes: u64) -> Result<(), String> {
    if facts.ram_bytes == 0 {
        return Ok(());
    }
    let have = facts.ram_bytes.saturating_add(facts.vram_bytes);
    let need = min_total_memory_for_model(model_bytes);
    if have < need {
        return Err(format!(
            "這台電腦的記憶體不足以執行本地模型（需要約 {}，這台電腦可用 {}）。\n\
             目前只提供這一款模型，沒有更小的版本可以換。\n\
             可以改用自訂 API 或 GPT 翻譯，那兩種不佔用本機記憶體，一樣能完成翻譯。",
            gb_text(need),
            gb_text(have)
        ));
    }
    Ok(())
}

pub fn bytes_needed_for_plan(plan: &InstallPlan) -> u64 {
    plan.total_bytes().saturating_add(SPACE_MARGIN_BYTES)
}

pub fn zip_for(manifest: &RemoteManifest, runtime: RuntimeKind) -> FileSpec {
    match runtime {
        RuntimeKind::Cuda => manifest.cuda.clone(),
        RuntimeKind::Vulkan => manifest.vulkan.clone(),
        RuntimeKind::Cpu => manifest.cpu.clone(),
    }
}

/// CUDA 載入失敗才允許再下 Vulkan；Vulkan 再失敗才下 CPU。不預抓另外兩套。
pub fn fallback_runtime(failed: RuntimeKind) -> Option<RuntimeKind> {
    match failed {
        RuntimeKind::Cuda => Some(RuntimeKind::Vulkan),
        RuntimeKind::Vulkan => Some(RuntimeKind::Cpu),
        RuntimeKind::Cpu => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gpu_probe_failure_must_not_fall_back_to_cpu_when_gpu_runtime_installed() {
        // 使用者實測：RX 7900 XT（20 GB）、已裝 llama-vulkan，但某次 WMI 偵測失敗
        // 就被當成沒有顯示卡，ngl 掉到 0，整包翻譯跑了快三小時。
        let failed_probe = HwFacts {
            ram_bytes: 32 * 1024 * 1024 * 1024,
            cpu_cores: 16,
            gpu_probe_failed: true,
            has_gpu: false, // 偵測失敗，所以這裡是 false
            ..Default::default()
        };
        assert_eq!(
            choose_profile(&failed_probe, RuntimeKind::Vulkan),
            HwProfile::Med,
            "已裝 GPU 執行套件時，偵測失敗不得退回 CPU"
        );
        assert_eq!(
            choose_profile(&failed_probe, RuntimeKind::Cuda),
            HwProfile::Med
        );
        // 真的是 CPU 版執行套件時，仍然照舊回 Cpu
        assert_eq!(choose_profile(&failed_probe, RuntimeKind::Cpu), HwProfile::Cpu);
    }

    #[test]
    fn confirmed_no_gpu_still_uses_cpu() {
        // 「確定沒有顯示卡」跟「查不到」的處置必須不同
        let no_gpu = HwFacts {
            ram_bytes: 16 * 1024 * 1024 * 1024,
            cpu_cores: 8,
            gpu_probe_failed: false,
            has_gpu: false,
            ..Default::default()
        };
        assert_eq!(choose_profile(&no_gpu, RuntimeKind::Cpu), HwProfile::Cpu);
    }

    #[test]
    fn amd_discrete_gpu_gets_vulkan_and_high_profile() {
        // 使用者的實際硬體：RX 7900 XT + 32 GB RAM
        let amd = HwFacts {
            ram_bytes: 32 * 1024 * 1024 * 1024,
            cpu_cores: 16,
            gpu_name: "AMD Radeon RX 7900 XT".into(),
            vram_bytes: 20 * 1024 * 1024 * 1024,
            nvidia: false,
            has_gpu: true,
            gpu_probe_failed: false,
        };
        assert_eq!(choose_runtime(&amd), RuntimeKind::Vulkan, "AMD 要走 Vulkan");
        assert_eq!(choose_profile(&amd, RuntimeKind::Vulkan), HwProfile::High);
    }

    fn spec(name: &str) -> FileSpec {
        FileSpec {
            name: name.into(),
            sha256: "a".repeat(64),
            bytes: 10,
        }
    }

    fn manifest() -> RemoteManifest {
        RemoteManifest {
            version: 1,
            model_id: "fixture-q4".into(),
            gguf: spec("model-q4.gguf"),
            cuda: spec("llama-cuda.zip"),
            vulkan: spec("llama-vulkan.zip"),
            cpu: spec("llama-cpu.zip"),
        }
    }

    #[test]
    fn nvidia_selects_only_cuda_zip() {
        let facts = HwFacts {
            nvidia: true,
            has_gpu: true,
            gpu_name: "NVIDIA GeForce RTX 4070".into(),
            vram_bytes: 12 * 1024 * 1024 * 1024,
            ram_bytes: 32 * 1024 * 1024 * 1024,
            cpu_cores: 8,
            gpu_probe_failed: false,
        };
        let plan = plan_from_manifest(&facts, &manifest()).unwrap();
        assert_eq!(plan.runtime, RuntimeKind::Cuda);
        assert_eq!(plan.zip.name, "llama-cuda.zip");
        let names: Vec<_> = plan.files().iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"model-q4.gguf"));
        assert!(!names.iter().any(|n| n.contains("vulkan") || n.contains("cpu")));
    }

    #[test]
    fn amd_selects_only_vulkan_zip() {
        let facts = HwFacts {
            nvidia: false,
            has_gpu: true,
            gpu_name: "AMD Radeon Graphics".into(),
            vram_bytes: 8 * 1024 * 1024 * 1024,
            ram_bytes: 16 * 1024 * 1024 * 1024,
            cpu_cores: 12,
            gpu_probe_failed: false,
        };
        let plan = plan_from_manifest(&facts, &manifest()).unwrap();
        assert_eq!(plan.runtime, RuntimeKind::Vulkan);
        assert_eq!(plan.zip.name, "llama-vulkan.zip");
        assert!(!plan.files().iter().any(|f| f.name.contains("cuda")));
    }

    #[test]
    fn intel_gpu_uses_vulkan_not_hardcoded_amd_sku() {
        let facts = HwFacts {
            nvidia: false,
            has_gpu: true,
            gpu_name: "Intel Arc A770".into(),
            vram_bytes: 16 * 1024 * 1024 * 1024,
            ram_bytes: 32 * 1024 * 1024 * 1024,
            cpu_cores: 16,
            gpu_probe_failed: false,
        };
        assert_eq!(choose_runtime(&facts), RuntimeKind::Vulkan);
        assert_ne!(facts.gpu_name.to_ascii_lowercase().as_str(), "rx 7900");
    }

    #[test]
    fn no_gpu_selects_only_cpu_zip() {
        let facts = HwFacts {
            nvidia: false,
            has_gpu: false,
            gpu_name: String::new(),
            ..HwFacts::default()
        };
        let plan = plan_from_manifest(&facts, &manifest()).unwrap();
        assert_eq!(plan.runtime, RuntimeKind::Cpu);
        assert_eq!(plan.profile, HwProfile::Cpu);
        assert_eq!(plan.zip.name, "llama-cpu.zip");
        assert_eq!(plan.files().len(), 2);
    }

    #[test]
    fn preflight_blocks_machines_that_cannot_possibly_load_the_model() {
        let seven_gb_model = 7_381_381_760u64;
        // 8 GB 記憶體載不動 7.4 GB 模型：要在下載前就說清楚，不是下載完才爆
        let small = HwFacts {
            ram_bytes: 8 * 1024 * 1024 * 1024,
            ..HwFacts::default()
        };
        let err = preflight(&small, seven_gb_model).unwrap_err();
        assert!(err.contains("記憶體"), "{err}");
        assert!(err.contains("自訂 API") || err.contains("GPT"), "必須給替代路徑：{err}");
        // 玩家文案不得出現內部術語
        assert!(!err.to_ascii_lowercase().contains("gguf"));
        assert!(!err.to_ascii_lowercase().contains("ngl"));

        // 32 GB 沒問題
        let big = HwFacts {
            ram_bytes: 32 * 1024 * 1024 * 1024,
            ..HwFacts::default()
        };
        assert!(preflight(&big, seven_gb_model).is_ok());

        // 讀不到記憶體（0）時放行，不能因為偵測失敗就擋人
        let unknown = HwFacts::default();
        assert!(preflight(&unknown, seven_gb_model).is_ok());
    }

    #[test]
    fn small_models_are_usable_on_small_machines() {
        // 尺寸階梯的前提：小模型在小機器上必須通過前置檢查。
        let small_machine = HwFacts {
            ram_bytes: 8 * 1024 * 1024 * 1024,
            ..HwFacts::default()
        };
        let small_model = 1_500_000_000u64; // ~1.4 GB
        assert!(preflight(&small_machine, small_model).is_ok());
        assert!(min_total_memory_for_model(small_model) < 8 * 1024 * 1024 * 1024);
    }

    #[test]
    fn vram_counts_towards_the_budget() {
        // 16 GB 記憶體 ＋ 20 GB 顯示記憶體跑 7.4 GB 模型完全沒問題，不可誤擋。
        let with_gpu = HwFacts {
            ram_bytes: 16 * 1024 * 1024 * 1024,
            vram_bytes: 20 * 1024 * 1024 * 1024,
            has_gpu: true,
            ..HwFacts::default()
        };
        assert!(preflight(&with_gpu, 7_381_381_760).is_ok());
    }

    #[test]
    fn unknown_vram_with_dgpu_does_not_fall_to_low() {
        let facts = HwFacts {
            nvidia: false,
            has_gpu: true,
            gpu_name: "AMD Radeon RX 7900 XT".into(),
            vram_bytes: 0, // 驅動與登錄檔都讀不到
            ram_bytes: 32 * 1024 * 1024 * 1024,
            cpu_cores: 16,
            gpu_probe_failed: false,
        };
        assert_eq!(choose_profile(&facts, RuntimeKind::Vulkan), HwProfile::Med);
        let thin = HwFacts {
            ram_bytes: 8 * 1024 * 1024 * 1024,
            ..facts.clone()
        };
        assert_eq!(choose_profile(&thin, RuntimeKind::Vulkan), HwProfile::Low);
    }

    #[test]
    fn fallback_only_after_cuda_or_vulkan() {
        assert_eq!(fallback_runtime(RuntimeKind::Cuda), Some(RuntimeKind::Vulkan));
        assert_eq!(fallback_runtime(RuntimeKind::Vulkan), Some(RuntimeKind::Cpu));
        assert_eq!(fallback_runtime(RuntimeKind::Cpu), None);
    }
}
