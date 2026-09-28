//! B4 第二輪 F7：從 gguf 檔頭讀模型總層數（`<架構>.block_count`）。
//!
//! 用途只有一個：估「部分放進顯示卡」時的速度比例。讀不到（格式不認得、檔案壞了）就回 `None`，
//! 呼叫端改用保守的純 CPU 速度假設——這裡絕不能讓啟動失敗。
//!
//! gguf 格式：魔數 `GGUF`、版本 u32、張量數 u64、鍵值數 u64，接著鍵值對
//! （鍵＝u64 長度＋UTF-8；型別 u32；值）。只讀到找到 `.block_count` 為止。

use std::fs::File;
use std::io::{BufReader, Read, Seek};
use std::path::Path;

/// 防呆上限：正常模型的鍵值數是幾十個、陣列（詞表）十幾萬筆。
const MAX_KV: u64 = 10_000;
const MAX_ARRAY_LEN: u64 = 50_000_000;
const MAX_STRING_LEN: u64 = 1 << 20;

pub fn read_block_count(path: &Path) -> Option<u32> {
    let file = File::open(path).ok()?;
    let mut r = BufReader::new(file);
    let mut magic = [0u8; 4];
    r.read_exact(&mut magic).ok()?;
    if &magic != b"GGUF" {
        return None;
    }
    let version = read_u32(&mut r)?;
    if version < 2 {
        return None; // v1 的長度欄位是 u32，不支援（現行模型都是 v3）
    }
    let _tensor_count = read_u64(&mut r)?;
    let kv_count = read_u64(&mut r)?;
    if kv_count > MAX_KV {
        return None;
    }
    for _ in 0..kv_count {
        let key = read_string(&mut r)?;
        let vtype = read_u32(&mut r)?;
        if key.ends_with(".block_count") {
            return match vtype {
                4 | 5 => read_u32(&mut r),
                10 | 11 => read_u64(&mut r).and_then(|v| u32::try_from(v).ok()),
                _ => None,
            };
        }
        skip_value(&mut r, vtype)?;
    }
    None
}

fn read_u32<R: Read>(r: &mut R) -> Option<u32> {
    let mut b = [0u8; 4];
    r.read_exact(&mut b).ok()?;
    Some(u32::from_le_bytes(b))
}

fn read_u64<R: Read>(r: &mut R) -> Option<u64> {
    let mut b = [0u8; 8];
    r.read_exact(&mut b).ok()?;
    Some(u64::from_le_bytes(b))
}

fn read_string<R: Read>(r: &mut R) -> Option<String> {
    let len = read_u64(r)?;
    if len > MAX_STRING_LEN {
        return None;
    }
    let mut buf = vec![0u8; len as usize];
    r.read_exact(&mut buf).ok()?;
    String::from_utf8(buf).ok()
}

fn fixed_size(vtype: u32) -> Option<i64> {
    Some(match vtype {
        0 | 1 | 7 => 1,
        2 | 3 => 2,
        4 | 5 | 6 => 4,
        10..=12 => 8,
        _ => return None,
    })
}

fn skip_value<R: Read + Seek>(r: &mut BufReader<R>, vtype: u32) -> Option<()> {
    if let Some(size) = fixed_size(vtype) {
        return r.seek_relative(size).ok();
    }
    match vtype {
        8 => {
            let len = read_u64(r)?;
            if len > MAX_STRING_LEN {
                return None;
            }
            r.seek_relative(len as i64).ok()
        }
        9 => {
            let elem = read_u32(r)?;
            let count = read_u64(r)?;
            if count > MAX_ARRAY_LEN {
                return None;
            }
            if let Some(size) = fixed_size(elem) {
                return r.seek_relative(size.checked_mul(count as i64)?).ok();
            }
            for _ in 0..count {
                skip_value(r, elem)?;
            }
            Some(())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(out: &mut Vec<u8>, s: &str) {
        out.extend((s.len() as u64).to_le_bytes());
        out.extend(s.as_bytes());
    }

    fn sample(block_count_type: u32) -> Vec<u8> {
        let mut out = b"GGUF".to_vec();
        out.extend(3u32.to_le_bytes());
        out.extend(0u64.to_le_bytes()); // tensors
        out.extend(4u64.to_le_bytes()); // kv
        string(&mut out, "general.architecture");
        out.extend(8u32.to_le_bytes());
        string(&mut out, "qwen3");
        string(&mut out, "tokenizer.ggml.tokens");
        out.extend(9u32.to_le_bytes());
        out.extend(8u32.to_le_bytes());
        out.extend(3u64.to_le_bytes());
        for t in ["a", "bb", "ccc"] {
            string(&mut out, t);
        }
        string(&mut out, "general.scores");
        out.extend(9u32.to_le_bytes());
        out.extend(6u32.to_le_bytes());
        out.extend(2u64.to_le_bytes());
        out.extend(1.0f32.to_le_bytes());
        out.extend(2.0f32.to_le_bytes());
        string(&mut out, "qwen3.block_count");
        out.extend(block_count_type.to_le_bytes());
        if block_count_type == 10 {
            out.extend(36u64.to_le_bytes());
        } else {
            out.extend(36u32.to_le_bytes());
        }
        out
    }

    fn write_tmp(tag: &str, bytes: &[u8]) -> std::path::PathBuf {
        let p = std::env::temp_dir().join(format!("mcpl-b4-gguf-{tag}-{}.gguf", std::process::id()));
        std::fs::write(&p, bytes).unwrap();
        p
    }

    #[test]
    fn reads_block_count_after_skipping_strings_and_arrays() {
        let p = write_tmp("ok", &sample(4));
        assert_eq!(read_block_count(&p), Some(36));
        let p64 = write_tmp("ok64", &sample(10));
        assert_eq!(read_block_count(&p64), Some(36));
        let _ = std::fs::remove_file(p);
        let _ = std::fs::remove_file(p64);
    }

    #[test]
    fn unreadable_files_return_none_instead_of_failing() {
        let bad = write_tmp("bad", b"NOTGGUF-garbage");
        assert_eq!(read_block_count(&bad), None);
        let mut truncated = sample(4);
        truncated.truncate(40);
        let t = write_tmp("trunc", &truncated);
        assert_eq!(read_block_count(&t), None);
        assert_eq!(read_block_count(Path::new("Z:/does/not/exist.gguf")), None);
        let _ = std::fs::remove_file(bad);
        let _ = std::fs::remove_file(t);
    }
}
