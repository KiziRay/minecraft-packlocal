//! Release manifest 的信任根（P0-07）。
//!
//! 舊版「PE 驗證」只是 `bytes.starts_with(b"MZ")` 兩位元組魔數，SHA-256 又來自
//! Worker 回應本身——等於下載來源自己保證自己。能同時控制回應與檔案的中間人
//! 可以無痛替換更新檔。本模組把信任根改成**編譯進 client 的 Ed25519 公鑰**。
//!
//! 硬規則：
//! 1. 公鑰**絕不**由 Worker 下發。下發的公鑰不是信任根。
//! 2. 私鑰永不進 repo、永不進 exe、永不進 build.rs env、永不進 Worker secret、永不進 log。
//!    簽章是獨立的人工步驟，只接受執行期傳入的私鑰路徑。
//! 3. `PINNED_KEYS` 為空時，stable 通道一律拒絕（見 `release_manifest`）——
//!    機制上線但正式發布自動被擋，不會出現「以為有簽其實沒有」。
//!
//! ## 輪替
//! 客戶端先認得新舊兩把 → 改用新金鑰簽 → 再發一版把舊金鑰標 Retiring／Revoked。
//!
//! ## 能力邊界（不得對外宣稱可遠端撤銷）
//! 撤銷清單編譯在 client 內，而 client 靠的正是被簽章保護的更新通道。
//! **私鑰外洩無法靠自動更新修復**——已散布的舊 client 仍會信任被盜金鑰。
//! 應變只能是站外公告加手動下載。

use base64::Engine as _;

/// `Active`／`Retiring` 目前在出貨程式碼裡沒有建構點，因為 `PINNED_KEYS` 還是空的
/// ——正式簽章金鑰尚未產生，這是本輪已登記的阻塞項。金鑰釘入後這個 allow 應一併移除。
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyStatus {
    Active,
    /// 仍可驗證既有 manifest，但不應再用來簽新版本。
    Retiring,
    /// 一律拒絕。
    Revoked,
}

pub struct PinnedKey {
    pub key_id: &'static str,
    pub public_key: [u8; 32],
    pub status: KeyStatus,
    /// ISO 8601 UTC。**只對 manifest 的 `releasedAt` 生效，不對 client 當下時鐘生效**——
    /// 否則舊 client 會因時鐘偏移永久失去更新能力。
    pub not_after: Option<&'static str>,
}

/// 正式發布金鑰**尚未產生**。空陣列會讓 stable 通道的驗證直接拒絕，
/// 這是刻意的阻擋：機制已上線，正式發布必須等真正的金鑰釘進來。
pub const PINNED_KEYS: &[PinnedKey] = &[];

pub fn find<'a>(keys: &'a [PinnedKey], key_id: &str) -> Option<&'a PinnedKey> {
    keys.iter().find(|k| k.key_id == key_id)
}

#[derive(Debug, PartialEq, Eq)]
pub enum SignatureError {
    NoPinnedKeys,
    UnknownKey(String),
    RevokedKey(String),
    KeyExpired(String),
    MalformedSignature,
    Mismatch,
}

/// 以釘選公鑰驗證 detached Ed25519 簽章。
///
/// `released_at` 用於 `not_after` 比對：ISO 8601 UTC 定寬字串的字典序等同時間序，
/// 因此直接字串比較即可，不需引入時間函式庫。
pub fn verify_detached(
    keys: &[PinnedKey],
    key_id: &str,
    signature_b64: &str,
    message: &[u8],
    released_at: &str,
) -> Result<(), SignatureError> {
    if keys.is_empty() {
        return Err(SignatureError::NoPinnedKeys);
    }
    let key = find(keys, key_id).ok_or_else(|| SignatureError::UnknownKey(key_id.to_string()))?;

    if key.status == KeyStatus::Revoked {
        return Err(SignatureError::RevokedKey(key_id.to_string()));
    }
    if let Some(not_after) = key.not_after {
        if released_at > not_after {
            return Err(SignatureError::KeyExpired(key_id.to_string()));
        }
    }

    let sig = base64::engine::general_purpose::STANDARD
        .decode(signature_b64.trim())
        .map_err(|_| SignatureError::MalformedSignature)?;
    if sig.len() != 64 {
        return Err(SignatureError::MalformedSignature);
    }

    ring::signature::UnparsedPublicKey::new(&ring::signature::ED25519, &key.public_key)
        .verify(message, &sig)
        .map_err(|_| SignatureError::Mismatch)
}

#[cfg(test)]
pub(crate) mod test_support {
    use super::*;

    /// 測試專用金鑰對。**永不出貨**：只在 `#[cfg(test)]` 編譯。
    /// 正式金鑰材料由使用者產生後才會釘進 `PINNED_KEYS`。
    pub const TEST_KEY_ID: &str = "test-ed25519-0001";

    pub fn keypair() -> (ring::signature::Ed25519KeyPair, [u8; 32]) {
        // 固定種子 → 固定金鑰對，測試才可重現。
        let seed = [7u8; 32];
        let pair = ring::signature::Ed25519KeyPair::from_seed_unchecked(&seed)
            .expect("test seed must build a key pair");
        let mut public = [0u8; 32];
        use ring::signature::KeyPair as _;
        public.copy_from_slice(pair.public_key().as_ref());
        (pair, public)
    }

    pub fn sign_b64(message: &[u8]) -> String {
        let (pair, _) = keypair();
        base64::engine::general_purpose::STANDARD.encode(pair.sign(message).as_ref())
    }

    pub fn pinned(status: KeyStatus, not_after: Option<&'static str>) -> Vec<PinnedKey> {
        let (_, public) = keypair();
        vec![PinnedKey { key_id: TEST_KEY_ID, public_key: public, status, not_after }]
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

    const RELEASED: &str = "2026-09-04T00:00:00Z";

    #[test]
    fn shipped_pinned_keys_are_empty_until_real_key_exists() {
        // 這條測試是刻意的阻擋：正式金鑰產生並釘入後，本測試會失敗，
        // 提醒同時更新 release 流程與本註解。
        assert!(
            PINNED_KEYS.is_empty(),
            "正式簽章金鑰已釘入；請同步更新發布流程與 Phase log 的阻塞清單"
        );
    }

    #[test]
    fn valid_signature_passes() {
        let keys = pinned(KeyStatus::Active, None);
        let msg = b"canonical manifest bytes";
        let sig = sign_b64(msg);
        assert_eq!(verify_detached(&keys, TEST_KEY_ID, &sig, msg, RELEASED), Ok(()));
    }

    #[test]
    fn tampered_message_is_rejected() {
        let keys = pinned(KeyStatus::Active, None);
        let sig = sign_b64(b"canonical manifest bytes");
        assert_eq!(
            verify_detached(&keys, TEST_KEY_ID, &sig, b"canonical manifest byteS", RELEASED),
            Err(SignatureError::Mismatch)
        );
    }

    #[test]
    fn empty_pinned_list_never_verifies() {
        let sig = sign_b64(b"x");
        assert_eq!(
            verify_detached(&[], TEST_KEY_ID, &sig, b"x", RELEASED),
            Err(SignatureError::NoPinnedKeys)
        );
    }

    #[test]
    fn unknown_and_revoked_keys_are_rejected() {
        let keys = pinned(KeyStatus::Active, None);
        let sig = sign_b64(b"x");
        assert_eq!(
            verify_detached(&keys, "someone-elses-key", &sig, b"x", RELEASED),
            Err(SignatureError::UnknownKey("someone-elses-key".into()))
        );

        let revoked = pinned(KeyStatus::Revoked, None);
        assert_eq!(
            verify_detached(&revoked, TEST_KEY_ID, &sig, b"x", RELEASED),
            Err(SignatureError::RevokedKey(TEST_KEY_ID.into()))
        );
    }

    #[test]
    fn not_after_applies_to_manifest_release_time_only() {
        let keys = pinned(KeyStatus::Retiring, Some("2026-09-01T00:00:00Z"));
        let sig = sign_b64(b"x");
        // manifest 發布時間晚於 not_after → 拒絕
        assert_eq!(
            verify_detached(&keys, TEST_KEY_ID, &sig, b"x", "2026-09-04T00:00:00Z"),
            Err(SignatureError::KeyExpired(TEST_KEY_ID.into()))
        );
        // 發布時間早於 not_after → 仍可驗證（Retiring 只是不再用來簽新版）
        assert_eq!(
            verify_detached(&keys, TEST_KEY_ID, &sig, b"x", "2026-08-01T00:00:00Z"),
            Ok(())
        );
    }

    #[test]
    fn malformed_signature_is_rejected() {
        let keys = pinned(KeyStatus::Active, None);
        assert_eq!(
            verify_detached(&keys, TEST_KEY_ID, "not-base64!!", b"x", RELEASED),
            Err(SignatureError::MalformedSignature)
        );
        // 長度不對的合法 base64 也要擋
        let short = base64::engine::general_purpose::STANDARD.encode([0u8; 10]);
        assert_eq!(
            verify_detached(&keys, TEST_KEY_ID, &short, b"x", RELEASED),
            Err(SignatureError::MalformedSignature)
        );
    }
}
