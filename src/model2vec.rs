use anyhow::{Context, Result, anyhow, bail};
use hf_hub::{Cache, Repo};
use ndarray::{Array1, Array2};
use safetensors::{SafeTensors, tensor::Dtype};
use serde_json::Value;
use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use tokenizers::Tokenizer;

pub trait Encoder: Send + Sync {
    fn encode(&self, texts: &[String]) -> Array2<f32>;
    fn dim(&self) -> usize;
}

pub const DEFAULT_MODEL_NAME: &str = "minishlab/potion-code-16M";
pub const DEFAULT_DIM: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModelLoadPolicy {
    AllowDownload,
    NoDownload,
    Offline,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelOptions {
    pub model: String,
    pub policy: ModelLoadPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EncoderSpec {
    Model2Vec(ModelOptions),
    Hashing { dim: usize },
}

impl Default for ModelOptions {
    fn default() -> Self {
        Self {
            model: default_model_name(),
            policy: ModelLoadPolicy::AllowDownload,
        }
    }
}

impl ModelOptions {
    pub fn new(model: Option<&str>, policy: ModelLoadPolicy) -> Self {
        Self {
            model: model
                .map(str::to_owned)
                .or_else(|| std::env::var("SIFS_MODEL").ok())
                .unwrap_or_else(default_model_name),
            policy,
        }
    }

    pub fn cache_key(&self) -> String {
        self.model
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
            .collect()
    }
}

impl EncoderSpec {
    pub fn model2vec(model: Option<&str>, policy: ModelLoadPolicy) -> Self {
        Self::Model2Vec(ModelOptions::new(model, policy))
    }

    pub fn hashing() -> Self {
        Self::Hashing { dim: DEFAULT_DIM }
    }

    pub fn cache_key(&self) -> String {
        match self {
            Self::Model2Vec(options) => options.cache_key(),
            Self::Hashing { dim } => format!("hashing-{dim}"),
        }
    }
}

pub fn default_model_name() -> String {
    std::env::var("SIFS_MODEL").unwrap_or_else(|_| DEFAULT_MODEL_NAME.to_owned())
}

pub fn load_model(model_path: Option<&str>) -> Result<Box<dyn Encoder>> {
    load_model_with_options(&ModelOptions::new(
        model_path,
        ModelLoadPolicy::AllowDownload,
    ))
}

pub fn load_model_with_options(options: &ModelOptions) -> Result<Box<dyn Encoder>> {
    if options.model == "__force_hashing_fallback__" {
        return Ok(Box::new(HashingEncoder::new(DEFAULT_DIM)));
    }
    Ok(Box::new(Model2VecEncoder::from_pretrained_with_options(
        options,
    )?))
}

pub fn load_encoder(spec: &EncoderSpec) -> Result<Box<dyn Encoder>> {
    match spec {
        EncoderSpec::Model2Vec(options) => load_model_with_options(options),
        EncoderSpec::Hashing { dim } => Ok(Box::new(HashingEncoder::new(*dim))),
    }
}

pub fn encoder_fingerprint(spec: &EncoderSpec) -> Result<String> {
    match spec {
        EncoderSpec::Model2Vec(options) => model_fingerprint(options),
        EncoderSpec::Hashing { dim } => Ok(format!("hashing-{dim}")),
    }
}

pub fn model_fingerprint(options: &ModelOptions) -> Result<String> {
    if options.model == "__force_hashing_fallback__" {
        return Ok("hashing-fallback-v1".to_owned());
    }
    let (tokenizer_path, model_path, config_path) = model_files(options)?;
    let mut hasher = DefaultHasher::new();
    options.model.hash(&mut hasher);
    hash_file(&tokenizer_path, &mut hasher)?;
    hash_file(&model_path, &mut hasher)?;
    hash_file(&config_path, &mut hasher)?;
    Ok(format!("{:016x}", hasher.finish()))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ModelStatus {
    pub model: String,
    pub tokenizer: Option<PathBuf>,
    pub safetensors: Option<PathBuf>,
    pub config: Option<PathBuf>,
}

impl ModelStatus {
    pub fn available(&self) -> bool {
        self.tokenizer.is_some() && self.safetensors.is_some() && self.config.is_some()
    }
}

pub fn model_status(model: Option<&str>) -> ModelStatus {
    let model = model
        .map(str::to_owned)
        .or_else(|| std::env::var("SIFS_MODEL").ok())
        .unwrap_or_else(default_model_name);
    let path = Path::new(&model);
    if path.exists() {
        let tokenizer = existing_file(path.join("tokenizer.json"));
        let safetensors = existing_file(path.join("model.safetensors"));
        let config = existing_file(path.join("config.json"));
        return ModelStatus {
            model,
            tokenizer,
            safetensors,
            config,
        };
    }
    let cache = Cache::from_env().repo(Repo::model(model.clone()));
    ModelStatus {
        model,
        tokenizer: cache.get("tokenizer.json"),
        safetensors: cache.get("model.safetensors"),
        config: cache.get("config.json"),
    }
}

pub struct Model2VecEncoder {
    tokenizer: Tokenizer,
    embeddings: Array2<f32>,
    weights: Option<Vec<f32>>,
    token_mapping: Option<Vec<usize>>,
    normalize: bool,
    median_token_length: usize,
    unk_token_id: Option<u32>,
}

impl Model2VecEncoder {
    pub fn from_pretrained(model_path: &str) -> Result<Self> {
        Self::from_pretrained_with_options(&ModelOptions::new(
            Some(model_path),
            ModelLoadPolicy::AllowDownload,
        ))
    }

    pub fn from_pretrained_with_options(options: &ModelOptions) -> Result<Self> {
        let (tokenizer_path, model_path, config_path) = model_files(options)?;
        let tokenizer = Tokenizer::from_file(&tokenizer_path)
            .map_err(|err| anyhow!("failed to load tokenizer: {err}"))?;
        let mut lens: Vec<usize> = tokenizer
            .get_vocab(false)
            .keys()
            .map(|token| token.len())
            .collect();
        lens.sort_unstable();
        let median_token_length = lens.get(lens.len() / 2).copied().unwrap_or(1);

        let config: Value = serde_json::from_reader(fs::File::open(config_path)?)?;
        let normalize = config
            .get("normalize")
            .and_then(Value::as_bool)
            .unwrap_or(true);
        let tokenizer_json: Value = serde_json::from_str(
            &tokenizer
                .to_string(false)
                .map_err(|err| anyhow!("failed to serialize tokenizer: {err}"))?,
        )?;
        let unk_token_id = validate_unk_token(&tokenizer, &tokenizer_json)?;

        let bytes = fs::read(model_path)?;
        let tensors = SafeTensors::deserialize(&bytes)?;
        let embeddings = read_matrix(&tensors, "embeddings")?;
        let weights = tensors
            .tensor("weights")
            .ok()
            .map(read_vector_f32)
            .transpose()?;
        let token_mapping = tensors
            .tensor("mapping")
            .ok()
            .map(read_vector_usize)
            .transpose()?;

        Ok(Self {
            tokenizer,
            embeddings,
            weights,
            token_mapping,
            normalize,
            median_token_length,
            unk_token_id,
        })
    }
}

impl Encoder for Model2VecEncoder {
    fn encode(&self, texts: &[String]) -> Array2<f32> {
        let dim = self.dim();
        let mut output = Array2::<f32>::zeros((texts.len(), dim));
        let truncated: Vec<String> = texts
            .iter()
            .map(|text| truncate_chars(text, 512 * self.median_token_length).to_owned())
            .collect();
        let encodings = self
            .tokenizer
            .encode_batch_fast::<String>(truncated, false)
            .expect("Model2Vec tokenizer encoding failed after model load validation");

        for (row_idx, encoding) in encodings.into_iter().enumerate() {
            let mut token_ids: Vec<u32> = encoding.get_ids().to_vec();
            if let Some(unk) = self.unk_token_id {
                token_ids.retain(|id| *id != unk);
            }
            token_ids.truncate(512);
            if token_ids.is_empty() {
                continue;
            }
            let mut count = 0usize;
            for id in token_ids {
                let token_idx = id as usize;
                let row = self
                    .token_mapping
                    .as_ref()
                    .and_then(|mapping| mapping.get(token_idx).copied())
                    .unwrap_or(token_idx);
                if row >= self.embeddings.nrows() {
                    continue;
                }
                let scale = self
                    .weights
                    .as_ref()
                    .and_then(|weights| weights.get(token_idx).copied())
                    .unwrap_or(1.0);
                let source = self.embeddings.row(row);
                for col in 0..dim {
                    output[(row_idx, col)] += source[col] * scale;
                }
                count += 1;
            }
            if count > 0 {
                for col in 0..dim {
                    output[(row_idx, col)] /= count as f32;
                }
            }
        }
        if self.normalize {
            for row in output.rows_mut() {
                normalize_row(row);
            }
        }
        output
    }

    fn dim(&self) -> usize {
        self.embeddings.ncols()
    }
}

fn model_files(options: &ModelOptions) -> Result<(PathBuf, PathBuf, PathBuf)> {
    let path = Path::new(&options.model);
    if path.exists() {
        return Ok((
            path.join("tokenizer.json"),
            path.join("model.safetensors"),
            path.join("config.json"),
        ));
    }
    if options.policy != ModelLoadPolicy::AllowDownload {
        let status = model_status(Some(&options.model));
        if let (Some(tokenizer), Some(model), Some(config)) =
            (status.tokenizer, status.safetensors, status.config)
        {
            return Ok((tokenizer, model, config));
        }
        bail!(
            "model {:?} is not available locally. Run `sifs model pull --model {}` or omit --offline/--no-download to allow download.",
            options.model,
            options.model
        );
    }
    let api = hf_hub::api::sync::Api::new()?;
    let repo = api.model(options.model.clone());
    Ok((
        repo.get("tokenizer.json")?,
        repo.get("model.safetensors")?,
        repo.get("config.json")?,
    ))
}

fn existing_file(path: PathBuf) -> Option<PathBuf> {
    path.exists().then_some(path)
}

fn validate_unk_token(tokenizer: &Tokenizer, tokenizer_json: &Value) -> Result<Option<u32>> {
    let Some(unk_token) = configured_unk_token(tokenizer_json) else {
        return Ok(None);
    };
    tokenizer.token_to_id(unk_token).map(Some).with_context(|| {
        format!("tokenizer unk_token {unk_token:?} is configured but missing from the vocabulary")
    })
}

fn configured_unk_token(tokenizer_json: &Value) -> Option<&str> {
    tokenizer_json
        .get("model")
        .and_then(|model| model.get("unk_token"))
        .and_then(Value::as_str)
}

fn hash_file(path: &Path, hasher: &mut DefaultHasher) -> Result<()> {
    let bytes = fs::read(path).with_context(|| format!("read model file {}", path.display()))?;
    path.file_name().hash(hasher);
    bytes.hash(hasher);
    Ok(())
}

fn read_matrix(tensors: &SafeTensors<'_>, name: &str) -> Result<Array2<f32>> {
    let tensor = tensors.tensor(name)?;
    let shape = tensor.shape();
    let [rows, cols]: [usize; 2] = shape.try_into().context("embedding tensor is not 2D")?;
    let values = read_f32_data(tensor.dtype(), tensor.data())?;
    Ok(Array2::from_shape_vec((rows, cols), values)?)
}

fn read_vector_f32(tensor: safetensors::tensor::TensorView<'_>) -> Result<Vec<f32>> {
    read_f32_data(tensor.dtype(), tensor.data())
}

fn read_f32_data(dtype: Dtype, raw: &[u8]) -> Result<Vec<f32>> {
    match dtype {
        Dtype::F32 => Ok(raw
            .chunks_exact(4)
            .map(|b| f32::from_le_bytes(b.try_into().unwrap()))
            .collect()),
        Dtype::F64 => Ok(raw
            .chunks_exact(8)
            .map(|b| f64::from_le_bytes(b.try_into().unwrap()) as f32)
            .collect()),
        Dtype::F16 => Ok(raw.chunks_exact(2).map(half_from_le_bytes).collect()),
        other => Err(anyhow!("unsupported float tensor dtype: {other:?}")),
    }
}

fn half_from_le_bytes(bytes: &[u8]) -> f32 {
    let bits = u16::from_le_bytes([bytes[0], bytes[1]]);
    half::f16::from_bits(bits).to_f32()
}

fn read_vector_usize(tensor: safetensors::tensor::TensorView<'_>) -> Result<Vec<usize>> {
    let raw = tensor.data();
    match tensor.dtype() {
        Dtype::I64 => Ok(raw
            .chunks_exact(8)
            .map(|b| i64::from_le_bytes(b.try_into().unwrap()) as usize)
            .collect()),
        Dtype::I32 => Ok(raw
            .chunks_exact(4)
            .map(|b| i32::from_le_bytes(b.try_into().unwrap()) as usize)
            .collect()),
        Dtype::U64 => Ok(raw
            .chunks_exact(8)
            .map(|b| u64::from_le_bytes(b.try_into().unwrap()) as usize)
            .collect()),
        Dtype::U32 => Ok(raw
            .chunks_exact(4)
            .map(|b| u32::from_le_bytes(b.try_into().unwrap()) as usize)
            .collect()),
        other => Err(anyhow!("unsupported mapping tensor dtype: {other:?}")),
    }
}

fn truncate_chars(text: &str, max_chars: usize) -> &str {
    match text.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &text[..byte_idx],
        None => text,
    }
}

pub struct HashingEncoder {
    dim: usize,
}

impl HashingEncoder {
    pub fn new(dim: usize) -> Self {
        Self { dim }
    }
}

impl Encoder for HashingEncoder {
    fn encode(&self, texts: &[String]) -> Array2<f32> {
        let mut matrix = Array2::<f32>::zeros((texts.len(), self.dim));
        for (row, text) in texts.iter().enumerate() {
            for tok in crate::tokens::tokenize(text) {
                let mut hasher = DefaultHasher::new();
                tok.hash(&mut hasher);
                let h = hasher.finish();
                let idx = (h as usize) % self.dim;
                let sign = if (h >> 63) == 0 { 1.0 } else { -1.0 };
                matrix[(row, idx)] += sign;
            }
            normalize_row(matrix.row_mut(row));
        }
        matrix
    }

    fn dim(&self) -> usize {
        self.dim
    }
}

fn normalize_row(mut row: ndarray::ArrayViewMut1<'_, f32>) {
    let norm = row.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 1e-8 {
        for v in row.iter_mut() {
            *v /= norm;
        }
    }
}

pub fn normalize_vector(mut vector: Array1<f32>) -> Array1<f32> {
    let norm = vector.iter().map(|v| v * v).sum::<f32>().sqrt();
    if norm > 1e-8 {
        vector.mapv_inplace(|v| v / norm);
    }
    vector
}

#[cfg(test)]
mod tests {
    use super::configured_unk_token;
    use serde_json::json;

    #[test]
    fn configured_unk_token_reads_model_setting_only_when_present() {
        assert_eq!(
            configured_unk_token(&json!({"model": {"unk_token": "[UNK]"}})),
            Some("[UNK]")
        );
        assert_eq!(configured_unk_token(&json!({"model": {}})), None);
    }
}
