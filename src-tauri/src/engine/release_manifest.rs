//! Release manifest 與更新驗證規則（P0-07 / P0-10）。
//!
//! 取代「Worker 回什麼就信什麼」的舊路徑。四道獨立關卡，任一不過即拒絕整份 manifest，
//! **不做「就近選一個」或部分採用**：
//!
//! 1. schema／格式：欄位齊全、URL 只准 https、檔名維持三段式（相容舊 client）。
//! 2. 來源身分：dirty 或 commit 不可追溯的建置**永遠不得**成為 stable 候選（B2-D）。
//! 3. 通道隔離：stable client 不會被 test／canary manifest 更新；同通道內拒絕降級。
//! 4. 信任根：Ed25519 釘選公鑰驗 canonical bytes；stable 沒有可驗證簽章一律拒絕。
//!
//! 刻意不提供 `signatureStatus` 這種「有沒有簽」的布林欄位——那種欄位由回應方自稱，
//! 不構成信任根。manifest 只有帶得出可驗證簽章才算簽過。

use serde::{Deserialize, Serialize};

use super::provenance::{eligible_for_stable, Channel};
use super::trust_keys::{self, PinnedKey, SignatureError};

pub const SUPPORTED_SCHEMA_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestProvenance {
    pub commit: String,
    pub dirty: bool,
    #[serde(rename = "buildTime")]
    pub build_time: String,
    /// `local` | `ci`。不得含使用者名稱或絕對路徑。
    pub builder: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestArtifact {
    pub name: String,
    pub url: String,
    pub sha256: String,
    pub bytes: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ManifestTrust {
    pub algorithm: String,
    #[serde(rename = "keyId")]
    pub key_id: String,
    pub signature: String,
    #[serde(rename = "authenticodeThumbprint", default, skip_serializing_if = "Option::is_none")]
    pub authenticode_thumbprint: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ManifestCompat {
    #[serde(rename = "minClientVersion", default)]
    pub min_client_version: String,
    #[serde(rename = "maxClientVersion", default, skip_serializing_if = "Option::is_none")]
    pub max_client_version: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReleaseManifest {
    #[serde(rename = "schemaVersion")]
    pub schema_version: u32,
    pub channel: String,
    pub version: String,
    #[serde(rename = "buildId")]
    pub build_id: String,
    #[serde(rename = "releasedAt")]
    pub released_at: String,
    pub provenance: ManifestProvenance,
    pub artifact: ManifestArtifact,
    /// 舊 Worker 不會回這個欄位；缺少即視為未簽章（stable 拒絕）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trust: Option<ManifestTrust>,
    #[serde(default)]
    pub compat: ManifestCompat,
}

pub struct ClientContext {
    pub version: String,
    pub build_id: String,
    pub channel: Channel,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SignatureState {
    /// 釘選公鑰驗證通過。
    Verified,
    /// 未簽章。只有非 stable 通道容許，UI **必須**標示。
    UnsignedNonStable,
}

#[derive(Debug, PartialEq, Eq)]
pub struct ManifestAccept {
    pub signature: SignatureState,
    /// 由測試通道收斂到 stable。必須以「切換通道」呈現，不得靜默更新。
    pub channel_switch: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ManifestReject {
    UnsupportedSchema(u32),
    UnknownChannel(String),
    InvalidTimestamp(String),
    InvalidVersion(String),
    /// B2-D：dirty 或來源不明的建置不得作為 stable 發布候選。
    DirtyOrUnknownBuildForStable,
    BadArtifactName(String),
    InsecureUrl(String),
    BadSha256,
    EmptyArtifact,
    ClientTooOld { required: String },
    ClientTooNew { maximum: String },
    ChannelMismatch { manifest: String, client: String },
    ChannelDowngrade { from: String, to: String },
    NotNewer { current: String, offered: String },
    /// stable 通道缺可驗證簽章。釘選清單為空時也走這裡。
    UnsignedStable,
    Signature(SignatureError),
}

impl ManifestReject {
    /// 給玩家看的白話原因。每個拒絕都要能解釋，不能只丟 enum 名稱。
    pub fn player_message(&self) -> String {
        match self {
            ManifestReject::UnsupportedSchema(v) => {
                format!("更新資訊格式版本 {v} 超出這個版本能處理的範圍，請手動下載最新版。")
            }
            ManifestReject::UnknownChannel(c) => format!("更新來源標示了未知的發布通道「{c}」，已停止更新。"),
            ManifestReject::InvalidTimestamp(t) => format!("更新資訊的發布時間「{t}」格式不正確，已停止更新。"),
            ManifestReject::InvalidVersion(v) => format!("更新資訊的版本號「{v}」格式不正確，已停止更新。"),
            ManifestReject::DirtyOrUnknownBuildForStable => {
                "這個更新檔的來源無法追溯（含未提交變更或缺少來源記錄），不可作為正式版發布。".to_string()
            }
            ManifestReject::BadArtifactName(n) => format!("更新檔名「{n}」不是官方 MCPL 免安裝 EXE 格式，已停止更新。"),
            ManifestReject::InsecureUrl(_) => "更新下載連結不是安全連線（https），已停止更新。".to_string(),
            ManifestReject::BadSha256 => "更新資訊缺少或帶有無效的檔案校驗碼，已停止更新。".to_string(),
            ManifestReject::EmptyArtifact => "更新資訊的檔案大小不合理，已停止更新。".to_string(),
            ManifestReject::ClientTooOld { required } => {
                format!("這個更新需要 {required} 或更新的版本才能安裝，請手動下載完整版。")
            }
            ManifestReject::ClientTooNew { maximum } => {
                format!("這個更新只適用到 {maximum} 版，你的版本較新，不需要安裝。")
            }
            ManifestReject::ChannelMismatch { manifest, client } => {
                format!("更新來自「{manifest}」通道，與你目前的「{client}」通道不符，已停止更新。")
            }
            ManifestReject::ChannelDowngrade { from, to } => {
                format!("不會把正式版（{from}）換成測試版（{to}）。要改用測試版請手動下載。")
            }
            ManifestReject::NotNewer { current, offered } => {
                format!("目前已是 {current}，伺服器提供的是 {offered}，不需要更新。")
            }
            ManifestReject::UnsignedStable => {
                "這個正式版更新沒有可驗證的官方簽章，已停止更新以保護你的電腦。".to_string()
            }
            ManifestReject::Signature(_) => "更新檔的官方簽章驗證失敗，已停止更新以保護你的電腦。".to_string(),
        }
    }
}

/// 簽章對象：manifest 移除 `trust.signature` 後的正規化 bytes。
///
/// 自行輸出而不依賴 serde_json 的 Map 型別，是為了避免 `preserve_order` 這類
/// feature unification 改變鍵序——簽章方與驗證方對「同一份 manifest」必須算出同一串 bytes。
pub fn canonical_bytes(manifest: &ReleaseManifest) -> Result<Vec<u8>, String> {
    let mut value =
        serde_json::to_value(manifest).map_err(|e| format!("manifest 無法序列化：{e}"))?;
    if let Some(trust) = value.get_mut("trust").and_then(|t| t.as_object_mut()) {
        trust.remove("signature");
    }
    Ok(canonical_json(&value).into_bytes())
}

fn canonical_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Null => "null".to_string(),
        serde_json::Value::Bool(b) => b.to_string(),
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::String(s) => {
            serde_json::to_string(s).unwrap_or_else(|_| "\"\"".to_string())
        }
        serde_json::Value::Array(items) => {
            let inner: Vec<String> = items.iter().map(canonical_json).collect();
            format!("[{}]", inner.join(","))
        }
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            let inner: Vec<String> = keys
                .into_iter()
                .map(|k| {
                    let key = serde_json::to_string(k).unwrap_or_else(|_| "\"\"".to_string());
                    format!("{key}:{}", canonical_json(&map[k]))
                })
                .collect();
            format!("{{{}}}", inner.join(","))
        }
    }
}

/// 舊版更新器只接受恰好三段的 `MCPL-x.y.z.exe`（見 1.0.8.1 事故）。
pub fn is_official_artifact_name(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    let Some(rest) = lower.strip_prefix("mcpl-") else {
        return false;
    };
    let Some(core) = rest.strip_suffix(".exe") else {
        return false;
    };
    parse_version(core).is_some()
}

/// 嚴格三段 semver。四段維護號屬於 `buildId`，不放 `version`。
pub fn parse_version(value: &str) -> Option<(u32, u32, u32)> {
    let parts: Vec<&str> = value.split('.').collect();
    if parts.len() != 3 {
        return None;
    }
    let mut out = [0u32; 3];
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() || !part.chars().all(|c| c.is_ascii_digit()) {
            return None;
        }
        out[i] = part.parse().ok()?;
    }
    Some((out[0], out[1], out[2]))
}

fn is_iso8601_utc(value: &str) -> bool {
    value.len() == 20
        && value.ends_with('Z')
        && value.as_bytes()[4] == b'-'
        && value.as_bytes()[7] == b'-'
        && value.as_bytes()[10] == b'T'
        && value.as_bytes()[13] == b':'
        && value.as_bytes()[16] == b':'
}

fn is_sha256_hex(value: &str) -> bool {
    value.len() == 64 && value.chars().all(|c| c.is_ascii_hexdigit())
}

/// 完整驗證。順序刻意由「格式」到「來源」到「通道」到「信任根」，
/// 讓失敗訊息指向最根本的原因而不是最後一關。
pub fn validate(
    manifest: &ReleaseManifest,
    client: &ClientContext,
    keys: &[PinnedKey],
) -> Result<ManifestAccept, ManifestReject> {
    // ── 1. schema 與格式 ─────────────────────────────
    if manifest.schema_version != SUPPORTED_SCHEMA_VERSION {
        return Err(ManifestReject::UnsupportedSchema(manifest.schema_version));
    }
    let channel = Channel::parse(&manifest.channel)
        .ok_or_else(|| ManifestReject::UnknownChannel(manifest.channel.clone()))?;
    if !is_iso8601_utc(&manifest.released_at) {
        return Err(ManifestReject::InvalidTimestamp(manifest.released_at.clone()));
    }
    let offered = parse_version(&manifest.version)
        .ok_or_else(|| ManifestReject::InvalidVersion(manifest.version.clone()))?;

    // ── 2. 來源身分：dirty／unknown 不得成為 stable 候選 ──
    if channel == Channel::Stable
        && !eligible_for_stable(&manifest.provenance.commit, manifest.provenance.dirty)
    {
        return Err(ManifestReject::DirtyOrUnknownBuildForStable);
    }

    // ── 3. artifact 安全性 ───────────────────────────
    if !is_official_artifact_name(&manifest.artifact.name) {
        return Err(ManifestReject::BadArtifactName(manifest.artifact.name.clone()));
    }
    if !manifest.artifact.url.to_ascii_lowercase().starts_with("https://") {
        return Err(ManifestReject::InsecureUrl(manifest.artifact.url.clone()));
    }
    if !is_sha256_hex(&manifest.artifact.sha256) {
        return Err(ManifestReject::BadSha256);
    }
    if manifest.artifact.bytes == 0 {
        return Err(ManifestReject::EmptyArtifact);
    }

    // ── 4. 相容範圍 ─────────────────────────────────
    let current = parse_version(&client.version)
        .ok_or_else(|| ManifestReject::InvalidVersion(client.version.clone()))?;
    if !manifest.compat.min_client_version.is_empty() {
        let min = parse_version(&manifest.compat.min_client_version)
            .ok_or_else(|| ManifestReject::InvalidVersion(manifest.compat.min_client_version.clone()))?;
        if current < min {
            return Err(ManifestReject::ClientTooOld {
                required: manifest.compat.min_client_version.clone(),
            });
        }
    }
    if let Some(max_raw) = manifest.compat.max_client_version.as_ref() {
        let max = parse_version(max_raw)
            .ok_or_else(|| ManifestReject::InvalidVersion(max_raw.clone()))?;
        if current > max {
            return Err(ManifestReject::ClientTooNew { maximum: max_raw.clone() });
        }
    }

    // ── 5. 通道隔離 ─────────────────────────────────
    let channel_switch = if channel == client.channel {
        // 同通道：嚴格拒絕降級。版本相同時，只有 buildId 不同的維護更新可放行。
        if offered < current
            || (offered == current && manifest.build_id.trim() == client.build_id.trim())
        {
            return Err(ManifestReject::NotNewer {
                current: client.version.clone(),
                offered: manifest.version.clone(),
            });
        }
        false
    } else if channel == Channel::Stable {
        // 收斂回正式線允許，但呼叫端必須以「切換通道」呈現。
        // 跨通道刻意不比版本高低，避免 test 的 1.0.9 蓋掉 stable 的 1.0.9。
        true
    } else if client.channel == Channel::Stable {
        return Err(ManifestReject::ChannelDowngrade {
            from: client.channel.as_str().to_string(),
            to: channel.as_str().to_string(),
        });
    } else {
        return Err(ManifestReject::ChannelMismatch {
            manifest: channel.as_str().to_string(),
            client: client.channel.as_str().to_string(),
        });
    };

    // ── 6. 信任根 ───────────────────────────────────
    let signature = match manifest.trust.as_ref() {
        None => {
            if channel == Channel::Stable {
                return Err(ManifestReject::UnsignedStable);
            }
            SignatureState::UnsignedNonStable
        }
        Some(trust) => {
            if keys.is_empty() && channel == Channel::Stable {
                return Err(ManifestReject::UnsignedStable);
            }
            let message = canonical_bytes(manifest)
                .map_err(|_| ManifestReject::Signature(SignatureError::MalformedSignature))?;
            match trust_keys::verify_detached(
                keys,
                &trust.key_id,
                &trust.signature,
                &message,
                &manifest.released_at,
            ) {
                Ok(()) => SignatureState::Verified,
                // 釘選清單為空且非 stable：機制尚未上線，視同未簽章而非驗證失敗。
                Err(SignatureError::NoPinnedKeys) => SignatureState::UnsignedNonStable,
                Err(e) => return Err(ManifestReject::Signature(e)),
            }
        }
    };

    Ok(ManifestAccept { signature, channel_switch })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::trust_keys::{test_support, KeyStatus};

    fn manifest(channel: &str, version: &str) -> ReleaseManifest {
        ReleaseManifest {
            schema_version: 1,
            channel: channel.to_string(),
            version: version.to_string(),
            build_id: format!("{version}+abcdefabcdef"),
            released_at: "2026-09-04T00:00:00Z".to_string(),
            provenance: ManifestProvenance {
                commit: "e66c7999b8432b2a4f370a62d78ef6abc211c8aa".to_string(),
                dirty: false,
                build_time: "2026-09-04T00:00:00Z".to_string(),
                builder: "local".to_string(),
            },
            artifact: ManifestArtifact {
                name: format!("MCPL-{version}.exe"),
                url: format!("https://example.invalid/download/MCPL-{version}.exe"),
                sha256: "a".repeat(64),
                bytes: 27_686_912,
            },
            trust: None,
            compat: ManifestCompat::default(),
        }
    }

    fn client(channel: Channel, version: &str) -> ClientContext {
        ClientContext {
            version: version.to_string(),
            build_id: format!("{version}+000000000000"),
            channel,
        }
    }

    fn sign(m: &mut ReleaseManifest) {
        m.trust = Some(ManifestTrust {
            algorithm: "ed25519-v1".to_string(),
            key_id: test_support::TEST_KEY_ID.to_string(),
            signature: String::new(),
            authenticode_thumbprint: None,
        });
        let msg = canonical_bytes(m).expect("canonical");
        if let Some(t) = m.trust.as_mut() {
            t.signature = test_support::sign_b64(&msg);
        }
    }

    // ───────── 信任根 ─────────

    #[test]
    fn signed_stable_manifest_is_accepted() {
        let mut m = manifest("stable", "1.1.0");
        sign(&mut m);
        let keys = test_support::pinned(KeyStatus::Active, None);
        let got = validate(&m, &client(Channel::Stable, "1.0.9"), &keys).expect("should accept");
        assert_eq!(got.signature, SignatureState::Verified);
        assert!(!got.channel_switch);
    }

    #[test]
    fn stable_without_signature_is_rejected() {
        let m = manifest("stable", "1.1.0");
        let keys = test_support::pinned(KeyStatus::Active, None);
        assert_eq!(
            validate(&m, &client(Channel::Stable, "1.0.9"), &keys),
            Err(ManifestReject::UnsignedStable)
        );
    }

    #[test]
    fn stable_with_empty_pinned_keys_is_rejected() {
        // 出貨常數陣列為空時，stable 必須被擋——這正是目前 PINNED_KEYS 的狀態。
        let mut m = manifest("stable", "1.1.0");
        sign(&mut m);
        assert_eq!(
            validate(&m, &client(Channel::Stable, "1.0.9"), &[]),
            Err(ManifestReject::UnsignedStable)
        );
    }

    #[test]
    fn tampered_manifest_fails_signature() {
        let mut m = manifest("stable", "1.1.0");
        sign(&mut m);
        // 簽完之後偷改下載位址
        m.artifact.url = "https://evil.invalid/download/MCPL-1.1.0.exe".to_string();
        let keys = test_support::pinned(KeyStatus::Active, None);
        assert_eq!(
            validate(&m, &client(Channel::Stable, "1.0.9"), &keys),
            Err(ManifestReject::Signature(SignatureError::Mismatch))
        );
    }

    #[test]
    fn revoked_key_is_rejected() {
        let mut m = manifest("stable", "1.1.0");
        sign(&mut m);
        let keys = test_support::pinned(KeyStatus::Revoked, None);
        assert_eq!(
            validate(&m, &client(Channel::Stable, "1.0.9"), &keys),
            Err(ManifestReject::Signature(SignatureError::RevokedKey(
                test_support::TEST_KEY_ID.into()
            )))
        );
    }

    #[test]
    fn signature_status_style_field_cannot_grant_trust() {
        // 模擬「舊 Worker 回一個自稱已簽的欄位」：schema 沒有 signatureStatus，
        // 反序列化會忽略它，manifest 仍然是未簽章 → stable 拒絕。
        let mut raw = serde_json::to_value(manifest("stable", "1.1.0")).unwrap();
        raw.as_object_mut()
            .unwrap()
            .insert("signatureStatus".into(), serde_json::Value::String("signed".into()));
        let parsed: ReleaseManifest = serde_json::from_value(raw).unwrap();
        assert!(parsed.trust.is_none());
        assert_eq!(
            validate(&parsed, &client(Channel::Stable, "1.0.9"), &[]),
            Err(ManifestReject::UnsignedStable)
        );
    }

    // ───────── B2-D：dirty／unknown ─────────

    #[test]
    fn dirty_build_cannot_be_stable() {
        let mut m = manifest("stable", "1.1.0");
        m.provenance.dirty = true;
        sign(&mut m);
        let keys = test_support::pinned(KeyStatus::Active, None);
        assert_eq!(
            validate(&m, &client(Channel::Stable, "1.0.9"), &keys),
            Err(ManifestReject::DirtyOrUnknownBuildForStable)
        );
    }

    #[test]
    fn unknown_commit_cannot_be_stable() {
        let mut m = manifest("stable", "1.1.0");
        m.provenance.commit = "unknown".to_string();
        sign(&mut m);
        let keys = test_support::pinned(KeyStatus::Active, None);
        assert_eq!(
            validate(&m, &client(Channel::Stable, "1.0.9"), &keys),
            Err(ManifestReject::DirtyOrUnknownBuildForStable)
        );
    }

    #[test]
    fn dirty_and_unknown_are_fine_on_test_channel() {
        // 同樣的組合在 test 通道允許——這正是 v28 那種測試建置該待的地方。
        let mut m = manifest("test", "1.1.0");
        m.provenance.dirty = true;
        m.provenance.commit = "unknown".to_string();
        let got = validate(&m, &client(Channel::Test, "1.0.9"), &[]).expect("test channel ok");
        assert_eq!(got.signature, SignatureState::UnsignedNonStable);
    }

    // ───────── 通道隔離 ─────────

    #[test]
    fn stable_client_rejects_test_manifest() {
        let m = manifest("test", "1.2.0");
        assert_eq!(
            validate(&m, &client(Channel::Stable, "1.0.9"), &[]),
            Err(ManifestReject::ChannelDowngrade {
                from: "stable".into(),
                to: "test".into()
            })
        );
    }

    #[test]
    fn cross_prerelease_channels_do_not_mix() {
        let m = manifest("canary", "1.2.0");
        assert_eq!(
            validate(&m, &client(Channel::Beta, "1.0.9"), &[]),
            Err(ManifestReject::ChannelMismatch {
                manifest: "canary".into(),
                client: "beta".into()
            })
        );
    }

    #[test]
    fn test_client_may_converge_to_stable() {
        let mut m = manifest("stable", "1.0.9");
        sign(&mut m);
        let keys = test_support::pinned(KeyStatus::Active, None);
        // 跨通道刻意不比版本高低：同版號也允許收斂，但必須標成切換通道。
        let got = validate(&m, &client(Channel::Test, "1.0.9"), &keys).expect("converge ok");
        assert!(got.channel_switch);
        assert_eq!(got.signature, SignatureState::Verified);
    }

    #[test]
    fn same_channel_downgrade_is_rejected() {
        let m = manifest("test", "1.0.8");
        assert_eq!(
            validate(&m, &client(Channel::Test, "1.0.9"), &[]),
            Err(ManifestReject::NotNewer {
                current: "1.0.9".into(),
                offered: "1.0.8".into()
            })
        );
    }

    #[test]
    fn same_version_same_build_is_not_an_update() {
        let mut m = manifest("test", "1.0.9");
        m.build_id = "1.0.9+aaaaaaaaaaaa".to_string();
        let c = ClientContext {
            version: "1.0.9".into(),
            build_id: "1.0.9+aaaaaaaaaaaa".into(),
            channel: Channel::Test,
        };
        assert_eq!(
            validate(&m, &c, &[]),
            Err(ManifestReject::NotNewer { current: "1.0.9".into(), offered: "1.0.9".into() })
        );
    }

    #[test]
    fn same_version_different_build_is_a_maintenance_update() {
        let mut m = manifest("test", "1.0.9");
        m.build_id = "1.0.9+bbbbbbbbbbbb".to_string();
        let c = ClientContext {
            version: "1.0.9".into(),
            build_id: "1.0.9+aaaaaaaaaaaa".into(),
            channel: Channel::Test,
        };
        assert!(validate(&m, &c, &[]).is_ok());
    }

    // ───────── 格式與安全 ─────────

    #[test]
    fn four_segment_artifact_name_is_rejected() {
        // 1.0.8.1 事故：四段檔名讓舊更新器整個失效。
        let mut m = manifest("test", "1.1.0");
        m.artifact.name = "MCPL-1.0.8.1.exe".to_string();
        assert_eq!(
            validate(&m, &client(Channel::Test, "1.0.9"), &[]),
            Err(ManifestReject::BadArtifactName("MCPL-1.0.8.1.exe".into()))
        );
        assert!(is_official_artifact_name("MCPL-1.0.9.exe"));
        assert!(!is_official_artifact_name("MCPL-1.0.8.1.exe"));
        assert!(!is_official_artifact_name("setup.exe"));
    }

    #[test]
    fn http_download_url_is_rejected() {
        let mut m = manifest("test", "1.1.0");
        m.artifact.url = "http://example.invalid/download/MCPL-1.1.0.exe".to_string();
        assert!(matches!(
            validate(&m, &client(Channel::Test, "1.0.9"), &[]),
            Err(ManifestReject::InsecureUrl(_))
        ));
    }

    #[test]
    fn bad_sha256_and_zero_bytes_are_rejected() {
        let mut m = manifest("test", "1.1.0");
        m.artifact.sha256 = "xyz".to_string();
        assert_eq!(validate(&m, &client(Channel::Test, "1.0.9"), &[]), Err(ManifestReject::BadSha256));

        let mut m2 = manifest("test", "1.1.0");
        m2.artifact.bytes = 0;
        assert_eq!(validate(&m2, &client(Channel::Test, "1.0.9"), &[]), Err(ManifestReject::EmptyArtifact));
    }

    #[test]
    fn unsupported_schema_and_channel_are_rejected() {
        let mut m = manifest("test", "1.1.0");
        m.schema_version = 2;
        assert_eq!(
            validate(&m, &client(Channel::Test, "1.0.9"), &[]),
            Err(ManifestReject::UnsupportedSchema(2))
        );

        let mut m2 = manifest("production", "1.1.0");
        m2.schema_version = 1;
        assert_eq!(
            validate(&m2, &client(Channel::Test, "1.0.9"), &[]),
            Err(ManifestReject::UnknownChannel("production".into()))
        );
    }

    #[test]
    fn compat_range_is_enforced() {
        let mut m = manifest("test", "1.2.0");
        m.compat.min_client_version = "1.1.0".to_string();
        assert_eq!(
            validate(&m, &client(Channel::Test, "1.0.9"), &[]),
            Err(ManifestReject::ClientTooOld { required: "1.1.0".into() })
        );
    }

    #[test]
    fn strict_semver_rejects_four_segments() {
        assert_eq!(parse_version("1.0.9"), Some((1, 0, 9)));
        assert_eq!(parse_version("1.0.8.1"), None);
        assert_eq!(parse_version("1.0"), None);
        assert_eq!(parse_version("1.0.x"), None);
        assert_eq!(parse_version(""), None);
    }

    // ───────── canonical bytes ─────────

    #[test]
    fn canonical_bytes_exclude_signature_and_are_key_sorted() {
        let mut m = manifest("test", "1.1.0");
        sign(&mut m);
        let bytes = canonical_bytes(&m).expect("canonical");
        let text = String::from_utf8(bytes).expect("utf8");
        assert!(!text.contains("\"signature\""), "簽章欄位不得包含在自己的簽章對象內");
        assert!(text.starts_with("{\"artifact\":"), "鍵序必須固定為字典序");
        assert!(!text.contains('\n') && !text.contains("  "), "不得有多餘空白");
    }

    #[test]
    fn canonical_bytes_are_stable_across_signature_changes() {
        let mut a = manifest("test", "1.1.0");
        let mut b = a.clone();
        sign(&mut a);
        b.trust = Some(ManifestTrust {
            algorithm: "ed25519-v1".to_string(),
            key_id: test_support::TEST_KEY_ID.to_string(),
            signature: "completely-different".to_string(),
            authenticode_thumbprint: None,
        });
        assert_eq!(canonical_bytes(&a).unwrap(), canonical_bytes(&b).unwrap());
    }

    #[test]
    fn every_reject_has_a_player_message() {
        let rejects = [
            ManifestReject::UnsupportedSchema(2),
            ManifestReject::UnknownChannel("x".into()),
            ManifestReject::InvalidTimestamp("x".into()),
            ManifestReject::InvalidVersion("x".into()),
            ManifestReject::DirtyOrUnknownBuildForStable,
            ManifestReject::BadArtifactName("x".into()),
            ManifestReject::InsecureUrl("x".into()),
            ManifestReject::BadSha256,
            ManifestReject::EmptyArtifact,
            ManifestReject::ClientTooOld { required: "1.1.0".into() },
            ManifestReject::ClientTooNew { maximum: "1.1.0".into() },
            ManifestReject::ChannelMismatch { manifest: "a".into(), client: "b".into() },
            ManifestReject::ChannelDowngrade { from: "a".into(), to: "b".into() },
            ManifestReject::NotNewer { current: "1".into(), offered: "0".into() },
            ManifestReject::UnsignedStable,
            ManifestReject::Signature(SignatureError::Mismatch),
        ];
        for r in &rejects {
            let msg = r.player_message();
            assert!(!msg.is_empty(), "{r:?} 缺少玩家可讀訊息");
            // 不得把 enum 名稱直接丟給玩家
            assert!(!msg.contains("Reject"), "{r:?} 的訊息像是內部代號");
        }
    }
}
