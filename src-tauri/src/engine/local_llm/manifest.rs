use super::select::FileSpec;
use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
struct RemoteFile {
    #[serde(default)]
    name: String,
    #[serde(default)]
    sha256: String,
    #[serde(default)]
    bytes: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct RemoteRuntimes {
    #[serde(default)]
    cuda: Option<RemoteFile>,
    #[serde(default)]
    vulkan: Option<RemoteFile>,
    #[serde(default)]
    cpu: Option<RemoteFile>,
}

#[derive(Debug, Deserialize)]
struct RemoteBody {
    #[serde(default)]
    version: u32,
    #[serde(default, rename = "modelId")]
    model_id: String,
    #[serde(default)]
    gguf: Option<RemoteFile>,
    #[serde(default)]
    runtimes: Option<RemoteRuntimes>,
}

fn to_spec(file: Option<RemoteFile>) -> super::select::FileSpec {
    let f = file.unwrap_or(RemoteFile {
        name: String::new(),
        sha256: String::new(),
        bytes: 0,
    });
    FileSpec {
        name: f.name.trim().to_string(),
        sha256: f.sha256.trim().to_ascii_lowercase(),
        bytes: f.bytes,
    }
}

pub fn parse_manifest_json(text: &str) -> Result<super::select::RemoteManifest, String> {
    let body: RemoteBody =
        serde_json::from_str(text).map_err(|e| format!("本地模型清單格式錯誤：{e}"))?;
    Ok(super::select::RemoteManifest {
        version: body.version,
        model_id: if body.model_id.trim().is_empty() {
            "local-q4".into()
        } else {
            body.model_id.trim().to_string()
        },
        gguf: to_spec(body.gguf),
        cuda: to_spec(body.runtimes.as_ref().and_then(|r| r.cuda.clone())),
        vulkan: to_spec(body.runtimes.as_ref().and_then(|r| r.vulkan.clone())),
        cpu: to_spec(body.runtimes.as_ref().and_then(|r| r.cpu.clone())),
    })
}

#[cfg(test)]
mod tests {
    use super::parse_manifest_json;

    #[test]
    fn parse_fixture_manifest() {
        let text = r#"{
            "version": 1,
            "modelId": "fixture-q4",
            "gguf": { "name": "model-q4.gguf", "sha256": "aa", "bytes": 16 },
            "runtimes": {
                "cuda": { "name": "llama-cuda.zip", "sha256": "bb", "bytes": 8 },
                "vulkan": { "name": "llama-vulkan.zip", "sha256": "cc", "bytes": 8 },
                "cpu": { "name": "llama-cpu.zip", "sha256": "dd", "bytes": 8 }
            }
        }"#;
        let m = parse_manifest_json(text).unwrap();
        assert_eq!(m.gguf.name, "model-q4.gguf");
        assert_eq!(m.cuda.name, "llama-cuda.zip");
        assert_eq!(m.cpu.name, "llama-cpu.zip");
    }
}
