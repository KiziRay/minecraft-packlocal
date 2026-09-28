//! 測試用的假 AI 伺服器（只在 `cargo test` 編譯）。
//!
//! B4 規定：測試一律用假伺服器，不打真實雲端 AI。每個請求照劇本回應，
//! 並記下收到幾次、每次的內容，讓測試能斷言「有沒有空轉重試」「有沒有拆半」。

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

/// 一次回應的劇本。
#[derive(Debug, Clone)]
pub struct FakeReply {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
    /// 回應前先等多久（模擬慢速模型／逾時）
    pub delay: Duration,
    /// 收到請求後直接斷線、不回任何東西（模擬程序當掉）
    pub hang_up: bool,
}

impl FakeReply {
    pub fn json(status: u16, body: impl Into<String>) -> Self {
        Self {
            status,
            headers: Vec::new(),
            body: body.into(),
            delay: Duration::ZERO,
            hang_up: false,
        }
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_string(), value.to_string()));
        self
    }

    pub fn delayed(mut self, delay: Duration) -> Self {
        self.delay = delay;
        self
    }

    pub fn hang_up() -> Self {
        Self {
            hang_up: true,
            ..Self::json(200, "")
        }
    }

    /// 標準的 chat completions 成功回應，`content` 是模型輸出的文字。
    pub fn chat(content: &str) -> Self {
        Self::chat_with_finish(content, "stop")
    }

    pub fn chat_with_finish(content: &str, finish: &str) -> Self {
        let body = serde_json::json!({
            "choices": [{
                "message": { "content": content },
                "finish_reason": finish,
            }],
            "usage": { "prompt_tokens": 10, "completion_tokens": 5 },
        });
        Self::json(200, body.to_string())
    }
}

type Handler = dyn Fn(usize, &str) -> FakeReply + Send + Sync;

pub struct FakeServer {
    pub base_url: String,
    hits: Arc<AtomicUsize>,
    bodies: Arc<Mutex<Vec<String>>>,
}

impl FakeServer {
    /// 啟動；`handler(第幾個請求（0 起算）, 請求本文)` 回傳劇本。
    pub fn start(handler: impl Fn(usize, &str) -> FakeReply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind(("127.0.0.1", 0)).expect("bind fake server");
        let port = listener.local_addr().unwrap().port();
        let hits = Arc::new(AtomicUsize::new(0));
        let bodies = Arc::new(Mutex::new(Vec::new()));
        let handler: Arc<Handler> = Arc::new(handler);
        {
            let hits = Arc::clone(&hits);
            let bodies = Arc::clone(&bodies);
            thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else { continue };
                    let hits = Arc::clone(&hits);
                    let bodies = Arc::clone(&bodies);
                    let handler = Arc::clone(&handler);
                    thread::spawn(move || serve_one(stream, &hits, &bodies, handler.as_ref()));
                }
            });
        }
        Self {
            base_url: format!("http://127.0.0.1:{port}"),
            hits,
            bodies,
        }
    }

    pub fn chat_url(&self) -> String {
        format!("{}/v1/chat/completions", self.base_url)
    }

    pub fn hits(&self) -> usize {
        self.hits.load(Ordering::SeqCst)
    }

    pub fn bodies(&self) -> Vec<String> {
        self.bodies.lock().map(|b| b.clone()).unwrap_or_default()
    }
}

fn serve_one(stream: TcpStream, hits: &AtomicUsize, bodies: &Mutex<Vec<String>>, handler: &Handler) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });
    let mut content_length = 0usize;
    let mut line = String::new();
    // 請求行＋標頭
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            break;
        }
        if let Some((name, value)) = trimmed.split_once(':') {
            if name.trim().eq_ignore_ascii_case("content-length") {
                content_length = value.trim().parse().unwrap_or(0);
            }
        }
    }
    let mut body = vec![0u8; content_length];
    if reader.read_exact(&mut body).is_err() {
        return;
    }
    let body = String::from_utf8_lossy(&body).to_string();
    let index = hits.fetch_add(1, Ordering::SeqCst);
    if let Ok(mut all) = bodies.lock() {
        all.push(body.clone());
    }
    let reply = handler(index, &body);
    if !reply.delay.is_zero() {
        thread::sleep(reply.delay);
    }
    let mut stream = stream;
    if reply.hang_up {
        let _ = stream.shutdown(std::net::Shutdown::Both);
        return;
    }
    let mut head = format!(
        "HTTP/1.1 {} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        reply.status,
        reply.body.len()
    );
    for (name, value) in &reply.headers {
        head.push_str(&format!("{name}: {value}\r\n"));
    }
    head.push_str("\r\n");
    let _ = stream.write_all(head.as_bytes());
    let _ = stream.write_all(reply.body.as_bytes());
    let _ = stream.flush();
}

/// 一個沒有人在聽的本機位址（連線會被拒絕）：模擬本地模型程序已經不在。
///
/// 用固定的特權埠 1：不能「借一個空埠再放掉」——平行執行的其他測試會拿到同一個埠，
/// 這個「死掉的伺服器」就突然活過來了。
pub fn dead_url() -> String {
    "http://127.0.0.1:1/v1/chat/completions".to_string()
}
