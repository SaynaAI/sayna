use anyhow::Result;
use tokenizers::tokenizer::Tokenizer as HfTokenizer;
use tokenizers::{Encoding, PaddingDirection, PaddingParams, PaddingStrategy, TruncationParams};
use tracing::{debug, info};

use crate::core::turn_detect::{assets, config::TurnDetectorConfig};
use ndarray::Array2;

pub struct Tokenizer {
    tokenizer: HfTokenizer,
    max_length: usize,
}

impl Tokenizer {
    pub async fn new(config: &TurnDetectorConfig) -> Result<Self> {
        let tokenizer_path = assets::tokenizer_path(config)?;

        info!("Loading tokenizer from: {:?}", tokenizer_path);

        // Move the blocking tokenizer loading to a dedicated blocking thread
        let max_length = config.max_sequence_length;
        let mut tokenizer = tokio::task::spawn_blocking({
            let tokenizer_path = tokenizer_path.clone();
            move || {
                HfTokenizer::from_file(&tokenizer_path)
                    .map_err(|e| anyhow::anyhow!("Failed to load tokenizer: {}", e))
            }
        })
        .await
        .map_err(|e| {
            anyhow::anyhow!("Failed to spawn blocking task for tokenizer loading: {}", e)
        })??;

        tokenizer.with_padding(Some(PaddingParams {
            strategy: PaddingStrategy::BatchLongest,
            direction: PaddingDirection::Right,
            pad_to_multiple_of: None,
            pad_id: 0,
            pad_type_id: 0,
            pad_token: "[PAD]".to_string(),
        }));

        tokenizer
            .with_truncation(Some(TruncationParams {
                max_length,
                strategy: tokenizers::TruncationStrategy::LongestFirst,
                stride: 0,
                direction: tokenizers::TruncationDirection::Right,
            }))
            .map_err(|e| anyhow::anyhow!("Failed to set truncation: {}", e))?;

        Ok(Self {
            tokenizer,
            max_length,
        })
    }

    /// Encode a single text string for turn detection
    pub async fn encode_single_text(&self, text: &str) -> Result<(Array2<i64>, Array2<i64>)> {
        debug!("Encoding text for turn detection: {}", text);

        let encoding = self
            .tokenizer
            .encode(text, false)
            .map_err(|e| anyhow::anyhow!("Failed to encode: {}", e))?;

        debug!("Encoded {} tokens", encoding.get_ids().len());

        let input_ids = self.get_input_ids(&encoding);
        let attention_mask = self.get_attention_mask(&encoding);

        let seq_len = input_ids.len();

        let input_ids_array = Array2::from_shape_vec((1, seq_len), input_ids)?;
        let attention_mask_array = Array2::from_shape_vec((1, seq_len), attention_mask)?;

        debug!(
            "Prepared input shape: {:?}, attention mask shape: {:?}",
            input_ids_array.shape(),
            attention_mask_array.shape()
        );

        Ok((input_ids_array, attention_mask_array))
    }

    pub fn get_input_ids(&self, encoding: &Encoding) -> Vec<i64> {
        encoding.get_ids().iter().map(|&id| id as i64).collect()
    }

    pub fn get_attention_mask(&self, encoding: &Encoding) -> Vec<i64> {
        encoding
            .get_attention_mask()
            .iter()
            .map(|&mask| mask as i64)
            .collect()
    }

    pub fn decode(&self, ids: &[u32], skip_special_tokens: bool) -> Result<String> {
        self.tokenizer
            .decode(ids, skip_special_tokens)
            .map_err(|e| anyhow::anyhow!("Failed to decode: {}", e))
    }

    pub fn max_length(&self) -> usize {
        self.max_length
    }
}
