//! multilingual-e5-small (Microsoft's E5, MIT): a 12-layer BERT with 384
//! dimensions and XLM-RoBERTa's 250k-token vocabulary, trained for
//! retrieval in about 100 languages. Run with candle on the CPU.
//!
//! E5 wants `"query: "` before a search and `"passage: "` before what's
//! searched, and its sentence vector is the mean of the token vectors.

use std::path::Path;

use candle_core::{D, DType, Device, Tensor};
use candle_nn::VarBuilder;
use candle_transformers::models::bert::{BertModel, Config};
use tokenizers::{PaddingParams, PaddingStrategy, Tokenizer, TruncationParams};

use crate::{EmbedError, Embedder, normalize};

/// The model's `config.json` from Hugging Face, compiled in rather than
/// downloaded: it never changes, and it's one fewer file to check.
const CONFIG: &str = r#"{
  "hidden_act": "gelu",
  "hidden_dropout_prob": 0.1,
  "hidden_size": 384,
  "initializer_range": 0.02,
  "intermediate_size": 1536,
  "layer_norm_eps": 1e-12,
  "max_position_embeddings": 512,
  "model_type": "bert",
  "num_attention_heads": 12,
  "num_hidden_layers": 12,
  "pad_token_id": 0,
  "position_embedding_type": "absolute",
  "type_vocab_size": 2,
  "use_cache": true,
  "classifier_dropout": null,
  "vocab_size": 250037
}"#;

/// Longest input the model reads, in tokens; longer text is cut. Quill's
/// chunks are well under it (about 120 words).
const MAX_TOKENS: usize = 512;

/// Passages embedded per forward pass. Sorted by length first, so padding
/// stays small.
const BATCH: usize = 16;

pub struct E5Embedder {
    model: BertModel,
    tokenizer: Tokenizer,
    device: Device,
    pool: rayon::ThreadPool,
}

impl E5Embedder {
    /// Stored with every vector; change it if the model, its prefixes or
    /// pooling change, so libraries index again.
    pub const ID: &'static str = "multilingual-e5-small";

    pub const DIMS: usize = 384;

    /// Loads the weights (`model.safetensors`) and `tokenizer.json`.
    /// Embedding runs on up to `threads` CPU cores.
    pub fn load(weights: &Path, tokenizer: &Path, threads: usize) -> Result<Self, EmbedError> {
        let device = Device::Cpu;
        let config: Config = serde_json::from_str(CONFIG)
            .map_err(|e| EmbedError::Model(candle_core::Error::Msg(e.to_string())))?;
        // Read into memory rather than memory-mapped (which needs `unsafe`
        // and would hold the file open, so Windows couldn't delete it).
        let tensors = candle_core::safetensors::load(weights, &device)?;
        let vb = VarBuilder::from_tensors(tensors, DType::F32, &device);
        let model = BertModel::load(vb, &config)?;

        let mut tokenizer =
            Tokenizer::from_file(tokenizer).map_err(|e| EmbedError::Tokenizer(e.to_string()))?;
        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length: MAX_TOKENS,
                ..TruncationParams::default()
            }))
            .map_err(|e| EmbedError::Tokenizer(e.to_string()))?;
        let pad_id = tokenizer.token_to_id("<pad>").unwrap_or(1);
        tokenizer.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            pad_id,
            pad_token: "<pad>".into(),
            ..PaddingParams::default()
        }));

        let pool = rayon::ThreadPoolBuilder::new()
            .num_threads(threads.max(1))
            .thread_name(|i| format!("quill-embed-{i}"))
            .build()
            .map_err(|e| EmbedError::Model(candle_core::Error::Msg(e.to_string())))?;
        Ok(Self {
            model,
            tokenizer,
            device,
            pool,
        })
    }

    /// Embeds one batch of already-prefixed texts.
    fn embed_batch(&self, texts: Vec<String>) -> Result<Vec<Vec<f32>>, EmbedError> {
        let encodings = self
            .tokenizer
            .encode_batch(texts, true)
            .map_err(|e| EmbedError::Tokenizer(e.to_string()))?;
        let rows = encodings.len();
        let len = encodings.first().map_or(0, tokenizers::Encoding::len);
        let ids: Vec<u32> = encodings
            .iter()
            .flat_map(|e| e.get_ids().to_vec())
            .collect();
        let mask: Vec<u32> = encodings
            .iter()
            .flat_map(|e| e.get_attention_mask().to_vec())
            .collect();
        let ids = Tensor::from_vec(ids, (rows, len), &self.device)?;
        let mask = Tensor::from_vec(mask, (rows, len), &self.device)?;
        let types = ids.zeros_like()?;
        let hidden = self.model.forward(&ids, &types, Some(&mask))?;
        // Mean of the token vectors, leaving out padding.
        let mask = mask.to_dtype(DType::F32)?.unsqueeze(D::Minus1)?;
        let summed = hidden.broadcast_mul(&mask)?.sum(1)?;
        let counts = mask.sum(1)?.clamp(1e-9, f32::MAX)?;
        let mean = summed.broadcast_div(&counts)?;
        let mut vectors: Vec<Vec<f32>> = mean.to_vec2()?;
        for v in &mut vectors {
            normalize(v);
        }
        Ok(vectors)
    }

    fn embed_prefixed(&self, prefix: &str, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        // Longest first, then batches of similar length.
        let mut order: Vec<usize> = (0..texts.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(texts[i].len()));
        let mut out = vec![Vec::new(); texts.len()];
        self.pool.install(|| {
            for batch in order.chunks(BATCH) {
                let prefixed = batch.iter().map(|&i| format!("{prefix}{}", texts[i]));
                let vectors = self.embed_batch(prefixed.collect())?;
                for (&i, v) in batch.iter().zip(vectors) {
                    out[i] = v;
                }
            }
            Ok::<_, EmbedError>(())
        })?;
        Ok(out)
    }
}

impl Embedder for E5Embedder {
    fn id(&self) -> &str {
        Self::ID
    }

    fn dims(&self) -> usize {
        Self::DIMS
    }

    fn embed_passages(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError> {
        self.embed_prefixed("passage: ", texts)
    }

    fn embed_query(&self, text: &str) -> Result<Vec<f32>, EmbedError> {
        Ok(self
            .embed_prefixed("query: ", &[text])?
            .pop()
            .unwrap_or_default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dot;

    /// Needs the model: `QUILL_TEST_EMBED_DIR=<models dir> cargo test -p
    /// quill-embed --release -- --ignored` (`quill-cli models download
    /// search` fetches it).
    #[test]
    #[ignore = "needs the embedding model (QUILL_TEST_EMBED_DIR)"]
    fn related_texts_score_higher_in_any_language() {
        let dir = std::path::PathBuf::from(std::env::var("QUILL_TEST_EMBED_DIR").unwrap());
        let e5 = E5Embedder::load(
            &dir.join("multilingual-e5-small.safetensors"),
            &dir.join("multilingual-e5-small-tokenizer.json"),
            4,
        )
        .unwrap();
        let passages = e5
            .embed_passages(&[
                "We're bringing on two more engineers before the end of the quarter.",
                "The database migration finished on Tuesday night.",
                "Vamos a contratar a dos ingenieros más este trimestre.",
            ])
            .unwrap();
        let q = e5.embed_query("when did we talk about hiring").unwrap();
        assert_eq!(q.len(), E5Embedder::DIMS);
        let scores: Vec<f32> = passages.iter().map(|p| dot(&q, p)).collect();
        assert!(scores[0] > scores[1], "{scores:?}");
        assert!(scores[2] > scores[1], "{scores:?}");
    }
}
