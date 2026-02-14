use anyhow::{anyhow, Result};
use ort::session::{builder::GraphOptimizationLevel, Session};
use ort::value::TensorRef;
use std::sync::{Mutex, Once};
use tokenizers::Tokenizer;

static ORT_INIT: Once = Once::new();

fn ensure_ort_init() {
    ORT_INIT.call_once(|| {
        // The system has an old onnxruntime.dll (v1.17) in System32.
        // We must load the pip-installed v1.24+ DLL explicitly before any ort API call.
        let dll_path = find_onnxruntime_dll().expect("Could not find onnxruntime.dll >= 1.23");
        ort::init_from(&dll_path)
            .expect("Failed to load onnxruntime DLL")
            .commit();
    });
}

fn ort_lib_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "onnxruntime.dll"
    } else if cfg!(target_os = "macos") {
        "libonnxruntime.dylib"
    } else {
        "libonnxruntime.so"
    }
}

fn find_onnxruntime_dll() -> Result<String> {
    let lib_name = ort_lib_name();

    // 1. ORT_DYLIB_PATH env var (highest priority, for dev/CI)
    if let Ok(path) = std::env::var("ORT_DYLIB_PATH") {
        if std::path::Path::new(&path).exists() {
            return Ok(path);
        }
    }

    // 2. Next to executable (production deployment)
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let candidate = dir.join(lib_name);
            if candidate.exists() {
                return Ok(candidate.to_string_lossy().to_string());
            }
        }
    }

    // 3. Python onnxruntime fallback (development only)
    let python_script = if cfg!(target_os = "windows") {
        "import onnxruntime, os; print(os.path.join(os.path.dirname(onnxruntime.__file__), 'capi', 'onnxruntime.dll'))"
    } else if cfg!(target_os = "macos") {
        "import onnxruntime, os; print(os.path.join(os.path.dirname(onnxruntime.__file__), 'capi', 'libonnxruntime.dylib'))"
    } else {
        "import onnxruntime, os; print(os.path.join(os.path.dirname(onnxruntime.__file__), 'capi', 'libonnxruntime.so'))"
    };

    let output = std::process::Command::new("python")
        .args(["-c", python_script])
        .output();
    if let Ok(out) = output {
        if out.status.success() {
            let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
            if std::path::Path::new(&path).exists() {
                return Ok(path);
            }
        }
    }

    Err(anyhow!(
        "ONNX Runtime library ({}) not found. Either:\n\
         1. Place {} next to the executable\n\
         2. Set ORT_DYLIB_PATH environment variable\n\
         3. Install: pip install onnxruntime",
        lib_name,
        lib_name
    ))
}

pub struct Embedder {
    session: Mutex<Session>,
    tokenizer: Tokenizer,
}

impl Embedder {
    pub fn load(model_path: &str, tokenizer_path: &str) -> Result<Self> {
        ensure_ort_init();
        let session = Session::builder()
            .map_err(|e| anyhow!("Failed to create session builder: {}", e))?
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(|e| anyhow!("Failed to set optimization level: {}", e))?
            .with_intra_threads(4)
            .map_err(|e| anyhow!("Failed to set threads: {}", e))?
            .commit_from_file(model_path)
            .map_err(|e| anyhow!("Failed to load ONNX model: {}", e))?;
        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow!("Failed to load tokenizer: {}", e))?;
        Ok(Self {
            session: Mutex::new(session),
            tokenizer,
        })
    }

    pub fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let encoding = self
            .tokenizer
            .encode(text, true)
            .map_err(|e| anyhow!("Tokenization failed: {}", e))?;

        let ids: Vec<i64> = encoding.get_ids().iter().map(|&id| id as i64).collect();
        let mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .map(|&m| m as i64)
            .collect();
        let seq_len = ids.len();

        let input_ids = TensorRef::from_array_view(([1usize, seq_len], &*ids))
            .map_err(|e| anyhow!("Failed to create input_ids tensor: {}", e))?;
        let attention_mask = TensorRef::from_array_view(([1usize, seq_len], &*mask))
            .map_err(|e| anyhow!("Failed to create attention_mask tensor: {}", e))?;

        let mut session = self
            .session
            .lock()
            .map_err(|e| anyhow!("Session lock poisoned: {}", e))?;
        let outputs = session
            .run(ort::inputs![input_ids, attention_mask])
            .map_err(|e| anyhow!("ONNX inference failed: {}", e))?;

        // outputs[0] = last_hidden_state [1, seq_len, 768]
        // outputs[1] = sentence_embedding [1, 768] — already pooled by the model
        let (_, embedding_view) = outputs[1]
            .try_extract_tensor::<f32>()
            .map_err(|e| anyhow!("Failed to extract embeddings: {}", e))?;

        let raw: Vec<f32> = embedding_view.iter().copied().collect();

        let norm: f32 = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm < 1e-12 {
            return Ok(raw);
        }
        let normalized: Vec<f32> = raw.iter().map(|x| x / norm).collect();

        Ok(normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get_embedder() -> Embedder {
        Embedder::load(
            "embedding_model/model.onnx",
            "embedding_model/tokenizer.json",
        )
        .expect("Failed to load embedder")
    }

    #[test]
    fn test_load() {
        let _embedder = get_embedder();
    }

    #[test]
    fn test_embed_basic() {
        let embedder = get_embedder();
        let vec = embedder.embed("hello world").unwrap();
        assert_eq!(vec.len(), 768);
    }

    #[test]
    fn test_embed_normalized() {
        let embedder = get_embedder();
        let vec = embedder.embed("hello world").unwrap();
        let norm: f32 = vec.iter().map(|x| x * x).sum::<f32>().sqrt();
        assert!(
            (norm - 1.0).abs() < 1e-4,
            "L2 norm should be ~1.0, got {}",
            norm
        );
    }

    #[test]
    fn test_embed_deterministic() {
        let embedder = get_embedder();
        let v1 = embedder.embed("test input").unwrap();
        let v2 = embedder.embed("test input").unwrap();
        assert_eq!(v1, v2);
    }

    #[test]
    fn test_embed_different() {
        let embedder = get_embedder();
        let v1 = embedder.embed("cat").unwrap();
        let v2 = embedder.embed("quantum physics").unwrap();
        let cosine: f32 = v1.iter().zip(v2.iter()).map(|(a, b)| a * b).sum();
        assert!(
            cosine < 0.9,
            "Different texts should have cosine sim < 0.9, got {}",
            cosine
        );
    }
}
