//! Text embeddings for search by meaning: the [`Embedder`] trait, and
//! [`E5Embedder`], which runs multilingual-e5-small on the CPU with candle
//! (pure Rust, so no second copy of onnxruntime next to sherpa-onnx's).
//!
//! An embedding is a list of numbers standing for what a text means: texts
//! about the same thing get vectors pointing the same way, whatever words
//! they use. Vectors here are normalised to length 1, so the cosine
//! similarity of two is their [`dot`] product.
//!
//! This crate knows nothing about meetings or files on disk: callers pass
//! model paths in and get vectors out (see `quill_core::semantic`).

mod e5;

pub use e5::E5Embedder;

#[derive(Debug, thiserror::Error)]
pub enum EmbedError {
    #[error("embedding model: {0}")]
    Model(#[from] candle_core::Error),
    #[error("tokenizer: {0}")]
    Tokenizer(String),
    #[error("{0}")]
    Io(#[from] std::io::Error),
}

/// Turns text into vectors. Behind a trait so the model or runtime can
/// change (or a fake stand in for tests) without touching the index.
pub trait Embedder: Send + Sync {
    /// Names the model and how it's used (prefixes, pooling). Stored next to
    /// every vector, so vectors from another model are never compared and
    /// changing it means indexing again.
    fn id(&self) -> &str;

    /// Length of each vector.
    fn dims(&self) -> usize;

    /// Vectors for passages to search through, normalised, in input order.
    fn embed_passages(&self, texts: &[&str]) -> Result<Vec<Vec<f32>>, EmbedError>;

    /// The vector for a search query, normalised. Models like E5 embed
    /// queries and passages differently.
    fn embed_query(&self, text: &str) -> Result<Vec<f32>, EmbedError>;
}

/// The dot product: cosine similarity for normalised vectors. Vectors of
/// different lengths compare only their common prefix (never happens for
/// one model).
pub fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

/// Scales `v` to length 1 (a zero vector stays zero).
pub fn normalize(v: &mut [f32]) {
    let norm = dot(v, v).sqrt();
    if norm > 0.0 {
        for x in v.iter_mut() {
            *x /= norm;
        }
    }
}

/// A vector as bytes for a SQLite BLOB: little-endian `f32`s.
pub fn to_bytes(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// The reverse of [`to_bytes`]; `None` if the length isn't a whole number
/// of `f32`s.
pub fn from_bytes(bytes: &[u8]) -> Option<Vec<f32>> {
    let (floats, rest) = bytes.as_chunks::<4>();
    if !rest.is_empty() {
        return None;
    }
    Some(floats.iter().map(|b| f32::from_le_bytes(*b)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bytes_round_trip() {
        let v = vec![0.25_f32, -1.5, 3.0e-7, 0.0];
        assert_eq!(from_bytes(&to_bytes(&v)).unwrap(), v);
        assert!(from_bytes(&[1, 2, 3]).is_none());
    }

    #[test]
    fn normalized_vectors_have_unit_dot() {
        let mut v = vec![3.0_f32, 4.0];
        normalize(&mut v);
        assert!((dot(&v, &v) - 1.0).abs() < 1e-6);
        assert!((v[0] - 0.6).abs() < 1e-6);
        let mut zero = vec![0.0_f32; 3];
        normalize(&mut zero);
        assert_eq!(zero, vec![0.0; 3]);
    }
}
