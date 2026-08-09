//! 全域取消旗標。
//!
//! 整合包翻譯動輒十幾分鐘。舊版唯一的中止方式是關掉視窗（而且「關閉＝縮小」預設開著，
//! 等於關不掉）。這裡提供一個所有階段都會檢查的旗標：掃描迴圈、AI 批次、覆寫寫檔。
//!
//! 語意：取消是**協作式**的——不會殺執行緒，只在下一個檢查點乾淨退出，
//! 已寫出的檔案保持完整。

use std::sync::atomic::{AtomicBool, Ordering};

static CANCELLED: AtomicBool = AtomicBool::new(false);

/// 使用者按下取消。
pub fn request() {
    CANCELLED.store(true, Ordering::SeqCst);
}

/// 每個長任務開始前呼叫，清掉上一輪的取消狀態。
pub fn reset() {
    CANCELLED.store(false, Ordering::SeqCst);
}

pub fn is_cancelled() -> bool {
    CANCELLED.load(Ordering::SeqCst)
}

/// 檢查點：已取消就回 `Err`，讓 `?` 直接把流程收掉。
pub fn check() -> Result<(), String> {
    if is_cancelled() {
        Err(CANCEL_MESSAGE.to_string())
    } else {
        Ok(())
    }
}

/// 前端據此判斷是「使用者取消」而非「翻譯失敗」。
pub const CANCEL_MESSAGE: &str = "已依你的要求停止；先前已完成的部分仍保留在結果資料夾。";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reset_clears_previous_request() {
        reset();
        assert!(!is_cancelled());
        request();
        assert!(is_cancelled());
        assert!(check().is_err());
        reset();
        assert!(check().is_ok());
    }
}
