//! Music Search Intent Classifier
//! ONNX Runtime inference for 3-class classification

use anyhow::{anyhow, Result};
use once_cell::sync::OnceCell;
use ort::session::Session;
use std::sync::{Arc, RwLock};
use tokenizers::Tokenizer;

static CLASSIFIER: OnceCell<Arc<RwLock<Classifier>>> = OnceCell::new();

const MAX_SEQ_LENGTH: usize = 512;

#[derive(Debug, Clone, Copy)]
pub enum SearchIntent {
    Artist(f32),
    Song(f32),
    Album(f32),
}

impl SearchIntent {
    pub fn label(&self) -> &'static str {
        match self {
            SearchIntent::Artist(_) => "Artist",
            SearchIntent::Song(_) => "Song",
            SearchIntent::Album(_) => "Album",
        }
    }

    pub fn confidence(&self) -> f32 {
        match self {
            SearchIntent::Artist(c) => *c,
            SearchIntent::Song(c) => *c,
            SearchIntent::Album(c) => *c,
        }
    }
}

pub struct Classifier {
    tokenizer: Tokenizer,
    session: Session,
    input_ids_name: String,
    attention_mask_name: String,
}

impl Classifier {
    pub fn new(model_path: &str, tokenizer_path: &str) -> Result<Self> {
        Self::with_config(model_path, tokenizer_path, "input_ids", "attention_mask")
    }

    pub fn with_config(
        model_path: &str,
        tokenizer_path: &str,
        input_ids_name: &str,
        attention_mask_name: &str,
    ) -> Result<Self> {
        let tokenizer = Tokenizer::from_file(tokenizer_path)
            .map_err(|e| anyhow!("Failed to load tokenizer: {}", e))?;

        let session = Session::builder()
            .map_err(|e| anyhow!("Failed to create session builder: {}", e))?
            .with_optimization_level(ort::session::builder::GraphOptimizationLevel::Level3)
            .map_err(|e| anyhow!("Failed to set optimization level: {}", e))?
            .with_intra_threads(2)
            .map_err(|e| anyhow!("Failed to set intra threads: {}", e))?
            .commit_from_file(model_path)
            .map_err(|e| anyhow!("Failed to load model: {}", e))?;

        Ok(Self {
            tokenizer,
            session,
            input_ids_name: input_ids_name.to_string(),
            attention_mask_name: attention_mask_name.to_string(),
        })
    }

    pub fn global() -> Option<&'static Arc<RwLock<Classifier>>> {
        CLASSIFIER.get()
    }

    pub fn init_global(model_path: &str, tokenizer_path: &str) -> Result<()> {
        Self::init_global_with_config(model_path, tokenizer_path, "input_ids", "attention_mask")
    }

    pub fn init_global_with_config(
        model_path: &str,
        tokenizer_path: &str,
        input_ids_name: &str,
        attention_mask_name: &str,
    ) -> Result<()> {
        let classifier = Self::with_config(
            model_path,
            tokenizer_path,
            input_ids_name,
            attention_mask_name,
        )?;
        CLASSIFIER
            .set(Arc::new(RwLock::new(classifier)))
            .map_err(|_| anyhow!("Classifier already initialized"))?;
        Ok(())
    }

    pub fn classify(&mut self, query: &str) -> Result<SearchIntent> {
        let encoding = self
            .tokenizer
            .encode(query, true)
            .map_err(|e| anyhow!("Tokenization failed: {}", e))?;

        let input_ids: Vec<i64> = encoding
            .get_ids()
            .iter()
            .take(MAX_SEQ_LENGTH)
            .map(|&x| x as i64)
            .collect();
        let attention_mask: Vec<i64> = encoding
            .get_attention_mask()
            .iter()
            .take(MAX_SEQ_LENGTH)
            .map(|&x| x as i64)
            .collect();

        let seq_len = input_ids.len();
        let mask_len = attention_mask.len();

        let outputs = self.session.run(ort::inputs![
            self.input_ids_name.as_str() => ort::value::Tensor::<i64>::from_array(([1, seq_len], input_ids.into_boxed_slice()))?,
            self.attention_mask_name.as_str() => ort::value::Tensor::<i64>::from_array(([1, mask_len], attention_mask.into_boxed_slice()))?,
        ]).map_err(|e| anyhow!("Session run failed: {}", e))?;

        let output = &outputs[0];
        let (_, logits_slice) = output.try_extract_tensor::<f32>()?;

        let max_logit = logits_slice
            .iter()
            .fold(f32::NEG_INFINITY, |a, b| a.max(*b));
        let exps: Vec<f32> = logits_slice
            .iter()
            .map(|&x| (x - max_logit).exp())
            .collect();
        let sum_exp: f32 = exps.iter().sum();
        let probabilities: Vec<f32> = if sum_exp > 0.0 {
            exps.iter().map(|&x| x / sum_exp).collect()
        } else {
            vec![0.0; exps.len()]
        };

        let (class_id, &confidence) = probabilities
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Less))
            .ok_or_else(|| anyhow!("No probabilities found"))?;

        let intent = match class_id {
            0 => SearchIntent::Artist(confidence),
            1 => SearchIntent::Song(confidence),
            2 => SearchIntent::Album(confidence),
            _ => unreachable!(),
        };

        Ok(intent)
    }
}

#[no_mangle]
/// Classify a search query string.
///
/// # Safety
/// The query pointer must be a valid null-terminated C string pointer.
/// The returned pointer is a newly allocated C string that must be freed with `free_classifier_string`.
pub unsafe extern "C" fn classify_search_query(
    query: *const std::os::raw::c_char,
) -> *mut std::os::raw::c_char {
    use std::ffi::{CStr, CString};

    let query = match CStr::from_ptr(query).to_string_lossy() {
        s if s.is_empty() => return std::ptr::null_mut(),
        s => s.into_owned(),
    };

    let classifier = match Classifier::global() {
        Some(c) => c,
        None => match CString::new(r#"{"error": "Classifier not initialized"}"#) {
            Ok(result) => return result.into_raw(),
            Err(_) => return std::ptr::null_mut(),
        },
    };

    let mut classifier = match classifier.write() {
        Ok(guard) => guard,
        Err(_) => {
            return match CString::new(r#"{"error": "Classifier locked"}"#) {
                Ok(result) => result.into_raw(),
                Err(_) => std::ptr::null_mut(),
            };
        }
    };

    match classifier.classify(&query) {
        Ok(intent) => {
            let result = serde_json::json!({
                "label": intent.label(),
                "confidence": intent.confidence(),
            });
            match CString::new(result.to_string()) {
                Ok(cstr) => cstr.into_raw(),
                Err(_) => std::ptr::null_mut(),
            }
        }
        Err(e) => {
            let error_json = serde_json::json!({ "error": e.to_string() }).to_string();
            match CString::new(error_json) {
                Ok(cstr) => cstr.into_raw(),
                Err(_) => std::ptr::null_mut(),
            }
        }
    }
}

#[no_mangle]
/// Free a string allocated by `classify_search_query`.
///
/// # Safety
/// The pointer must have been allocated by `classify_search_query` and not previously freed.
pub unsafe extern "C" fn free_classifier_string(s: *mut std::os::raw::c_char) {
    if !s.is_null() {
        let _ = std::ffi::CString::from_raw(s);
    }
}
