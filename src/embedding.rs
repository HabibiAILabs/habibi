use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Mutex,
};

use anyhow::{Context, Result, bail, ensure};
use fastembed::{
    InitOptionsUserDefined, Pooling, QuantizationMode, TextEmbedding, TokenizerFiles,
    UserDefinedEmbeddingModel,
};
use reqwest::blocking::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::{event::Event, store::SharedEventStore, tool::ToolDefinition};

pub const EMBEDDING_MODEL_ID: &str = "BAAI/bge-small-en-v1.5";
pub const EMBEDDING_REPOSITORY: &str = "Qdrant/bge-small-en-v1.5-onnx-Q";
pub const EMBEDDING_REVISION: &str = "52398278842ec682c6f32300af41344b1c0b0bb2";
pub const EMBEDDING_DIMENSIONS: usize = 384;
pub const EMBEDDING_MAX_TOKENS: usize = 512;
pub const EMBEDDING_QUERY_PREFIX: &str =
    "Represent this sentence for searching relevant passages: ";
pub const EMBEDDING_MODEL_KEY: &str =
    "bge-small-en-v1.5-onnx-q-52398278842ec682c6f32300af41344b1c0b0bb2";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub struct ModelFileManifest {
    pub name: &'static str,
    pub size: u64,
    pub sha256: &'static str,
}

pub const MODEL_FILES: &[ModelFileManifest] = &[
    ModelFileManifest {
        name: "model_optimized.onnx",
        size: 66_465_124,
        sha256: "51f1bd0addd6e859e42c2c8021a5e5461385bb676a649f4b269aa445449f2431",
    },
    ModelFileManifest {
        name: "tokenizer.json",
        size: 711_396,
        sha256: "d241a60d5e8f04cc1b2b3e9ef7a4921b27bf526d9f6050ab90f9267a1f9e5c66",
    },
    ModelFileManifest {
        name: "config.json",
        size: 706,
        sha256: "13582bcf2effc85b7bf3d3f5532e686bc1c9ce86bb009d10f0ec33cbe92299dd",
    },
    ModelFileManifest {
        name: "special_tokens_map.json",
        size: 695,
        sha256: "5d5b662e421ea9fac075174bb0688ee0d9431699900b90662acd44b2a350503a",
    },
    ModelFileManifest {
        name: "tokenizer_config.json",
        size: 1_242,
        sha256: "0b29c7bfc889e53b36d9dd3e686dd4300f6525110eaa98c76a5dafceb2029f53",
    },
];

pub trait Embedder: Send + Sync {
    fn model_id(&self) -> &str;
    fn revision(&self) -> &str;
    fn dimensions(&self) -> usize;
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>>;
}

#[cfg(test)]
pub struct DeterministicTestEmbedder;

#[cfg(test)]
impl Embedder for DeterministicTestEmbedder {
    fn model_id(&self) -> &str {
        "test-embedding"
    }
    fn revision(&self) -> &str {
        "1"
    }
    fn dimensions(&self) -> usize {
        4
    }
    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|text| {
                let digest = Sha256::digest(text.as_bytes());
                let mut vector = (0..4)
                    .map(|index| {
                        u16::from_le_bytes([digest[index * 2], digest[index * 2 + 1]]) as f32
                            / u16::MAX as f32
                    })
                    .collect::<Vec<_>>();
                let norm = vector.iter().map(|value| value * value).sum::<f32>().sqrt();
                vector.iter_mut().for_each(|value| *value /= norm);
                vector
            })
            .collect())
    }
}

pub struct LocalEmbedder {
    model: Mutex<TextEmbedding>,
}

impl LocalEmbedder {
    pub fn load_default() -> Result<Self> {
        Self::load(&model_dir()?)
    }

    pub fn load(path: &Path) -> Result<Self> {
        validate_model_dir(path)?;
        let read = |name: &str| {
            fs::read(path.join(name))
                .with_context(|| format!("failed to read embedding model file '{name}'"))
        };
        let tokenizer_files = TokenizerFiles {
            tokenizer_file: read("tokenizer.json")?,
            config_file: read("config.json")?,
            special_tokens_map_file: read("special_tokens_map.json")?,
            tokenizer_config_file: read("tokenizer_config.json")?,
        };
        let model = UserDefinedEmbeddingModel::new(read("model_optimized.onnx")?, tokenizer_files)
            .with_pooling(Pooling::Cls)
            .with_quantization(QuantizationMode::Static);
        let options = InitOptionsUserDefined::new()
            .with_max_length(EMBEDDING_MAX_TOKENS)
            .with_intra_threads(2);
        let model = TextEmbedding::try_new_from_user_defined(model, options)
            .context("failed to initialize the pinned local embedding model")?;
        Ok(Self {
            model: Mutex::new(model),
        })
    }
}

impl Embedder for LocalEmbedder {
    fn model_id(&self) -> &str {
        EMBEDDING_MODEL_ID
    }

    fn revision(&self) -> &str {
        EMBEDDING_REVISION
    }

    fn dimensions(&self) -> usize {
        EMBEDDING_DIMENSIONS
    }

    fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
        let embeddings = self
            .model
            .lock()
            .map_err(|_| anyhow::anyhow!("embedding model lock poisoned"))?
            .embed(texts, Some(32))
            .context("local embedding inference failed")?;
        ensure!(
            embeddings
                .iter()
                .all(|value| value.len() == EMBEDDING_DIMENSIONS),
            "embedding model returned an unexpected vector size"
        );
        Ok(embeddings)
    }
}

pub fn model_dir() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("HABIBI_EMBEDDING_DIR") {
        return Ok(PathBuf::from(path));
    }
    let cache = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
        .context("HOME or XDG_CACHE_HOME is required for the embedding model cache")?;
    Ok(cache
        .join("habibi")
        .join("embeddings")
        .join(EMBEDDING_MODEL_KEY))
}

pub fn install_default_model() -> Result<PathBuf> {
    let target = model_dir()?;
    if target.exists() {
        if validate_model_dir(&target).is_ok() {
            return Ok(target);
        }
        fs::remove_dir_all(&target).context("failed to replace corrupt embedding model cache")?;
    }
    let parent = target
        .parent()
        .context("embedding model path has no parent directory")?;
    fs::create_dir_all(parent)?;
    let staging = tempfile::Builder::new()
        .prefix(".embedding-install-")
        .tempdir_in(parent)?;
    let client = Client::builder()
        .user_agent(concat!("habibi/", env!("CARGO_PKG_VERSION")))
        .build()?;
    for file in MODEL_FILES {
        let url = format!(
            "https://huggingface.co/{}/resolve/{}/{}",
            EMBEDDING_REPOSITORY, EMBEDDING_REVISION, file.name
        );
        let mut response = client.get(&url).send()?.error_for_status()?;
        let output_path = staging.path().join(file.name);
        let mut output = fs::File::create(&output_path)?;
        let mut hasher = Sha256::new();
        let mut size = 0_u64;
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let read = response.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            output.write_all(&buffer[..read])?;
            hasher.update(&buffer[..read]);
            size += read as u64;
            ensure!(
                size <= file.size,
                "downloaded {} exceeds its pinned size",
                file.name
            );
        }
        output.sync_all()?;
        ensure!(
            size == file.size,
            "downloaded {} has an invalid size",
            file.name
        );
        ensure!(
            format!("{:x}", hasher.finalize()) == file.sha256,
            "downloaded {} failed SHA-256 verification",
            file.name
        );
    }
    validate_model_dir(staging.path())?;
    let staging_path = staging.keep();
    if let Err(error) = fs::rename(&staging_path, &target) {
        let _ = fs::remove_dir_all(&staging_path);
        if target.exists() {
            validate_model_dir(&target)?;
        } else {
            return Err(error).context("failed to atomically install embedding model");
        }
    }
    Ok(target)
}

pub fn validate_model_dir(path: &Path) -> Result<()> {
    if !path.is_dir() {
        bail!(
            "embedding model is not installed at '{}'; run 'habibi embeddings install'",
            path.display()
        );
    }
    for file in MODEL_FILES {
        let path = path.join(file.name);
        let metadata = fs::metadata(&path).with_context(|| {
            format!(
                "embedding model is incomplete at '{}'; run 'habibi embeddings install'",
                path.display()
            )
        })?;
        ensure!(
            metadata.len() == file.size,
            "embedding model file '{}' has an invalid size",
            file.name
        );
        let digest = sha256_file(&path)?;
        ensure!(
            digest == file.sha256,
            "embedding model file '{}' failed SHA-256 verification",
            file.name
        );
    }
    Ok(())
}

fn sha256_file(path: &Path) -> Result<String> {
    let mut input = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut input, &mut hasher_writer(&mut hasher))?;
    Ok(format!("{:x}", hasher.finalize()))
}

fn hasher_writer<'a>(hasher: &'a mut Sha256) -> impl Write + 'a {
    struct Writer<'a>(&'a mut Sha256);
    impl Write for Writer<'_> {
        fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
            self.0.update(buffer);
            Ok(buffer.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }
    Writer(hasher)
}

pub const SEMANTIC_TOOL_LIMIT: usize = 50;
pub const USED_TOOL_LIMIT: usize = 50;
pub const FINAL_TOOL_LIMIT: usize = 12;
pub const MIN_TOOL_SIMILARITY: f32 = 0.50;
pub const SEMANTIC_EVENT_LIMIT: usize = 20;
pub const SEMANTIC_EVENT_CANDIDATE_LIMIT: usize = 10_000;
pub const MIN_EVENT_SIMILARITY: f32 = 0.50;

#[derive(Debug, Clone, Serialize)]
pub struct SemanticToolMatch {
    pub tool: String,
    pub score: f32,
    pub rank: usize,
}

#[derive(Clone)]
struct IndexedTool {
    name: String,
    vector: Vec<f32>,
}

#[derive(Default)]
struct IndexState {
    catalogs: std::collections::HashMap<String, Vec<IndexedTool>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticEventMatch {
    pub event: crate::event::StoredEvent,
    pub score: f32,
    pub rank: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct SemanticEventSearchResult {
    pub embedding_model: String,
    pub embedding_revision: String,
    pub candidates_scanned: usize,
    pub matches: Vec<SemanticEventMatch>,
}

pub struct EventEmbeddingIndex {
    embedder: std::sync::Arc<dyn Embedder>,
    store: SharedEventStore,
}

impl EventEmbeddingIndex {
    pub fn new(embedder: std::sync::Arc<dyn Embedder>, store: SharedEventStore) -> Self {
        Self { embedder, store }
    }

    pub fn search(
        &self,
        text: &str,
        before_sequence: i64,
        limit: usize,
        minimum_similarity: f32,
    ) -> Result<SemanticEventSearchResult> {
        ensure!(
            text.len() <= EVENT_TOOL_QUERY_BYTES,
            "semantic event query exceeds 16 KiB"
        );
        ensure!(
            minimum_similarity.is_finite(),
            "minimum similarity must be finite"
        );
        let limit = limit.clamp(1, SEMANTIC_EVENT_LIMIT);
        let mut candidates = self
            .store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .event_embedding_candidates(
                before_sequence,
                SEMANTIC_EVENT_CANDIDATE_LIMIT,
                self.embedder.model_id(),
                self.embedder.revision(),
                self.embedder.dimensions(),
            )?;
        let candidates_scanned = candidates.len();
        let documents = candidates
            .iter()
            .map(|candidate| {
                let document = canonical_event_text(&candidate.event.event);
                let hash = format!("{:x}", Sha256::digest(document.as_bytes()));
                (document, hash)
            })
            .collect::<Vec<_>>();
        let missing_documents = candidates
            .iter()
            .zip(&documents)
            .filter(|(candidate, (_, hash))| {
                candidate.vector.is_none()
                    || candidate.document_sha256.as_deref() != Some(hash.as_str())
            })
            .map(|(_, (document, _))| document.clone())
            .collect::<Vec<_>>();
        let mut generated = if missing_documents.is_empty() {
            Vec::new()
        } else {
            self.embedder.embed(&missing_documents)?
        }
        .into_iter();
        for (candidate, (_, hash)) in candidates.iter_mut().zip(&documents) {
            if candidate.vector.is_none()
                || candidate.document_sha256.as_deref() != Some(hash.as_str())
            {
                let vector = generated
                    .next()
                    .context("embedding model omitted an event vector")?;
                self.store
                    .lock()
                    .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
                    .save_event_embedding(
                        candidate.event.event.id,
                        hash,
                        self.embedder.model_id(),
                        self.embedder.revision(),
                        self.embedder.dimensions(),
                        &vector,
                    )?;
                candidate.document_sha256 = Some(hash.clone());
                candidate.vector = Some(vector);
            }
        }
        ensure!(
            generated.next().is_none(),
            "embedding model returned too many event vectors"
        );
        let query = self
            .embedder
            .embed(&[format!("{EMBEDDING_QUERY_PREFIX}{text}")])?
            .into_iter()
            .next()
            .context("embedding model omitted the event query vector")?;
        let mut matches = candidates
            .into_iter()
            .filter_map(|candidate| {
                let score = normalized_dot(&query, candidate.vector.as_ref()?).ok()?;
                (score >= minimum_similarity).then_some((candidate.event, score))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|(left, left_score), (right, right_score)| {
            right_score
                .total_cmp(left_score)
                .then_with(|| right.sequence.cmp(&left.sequence))
                .then_with(|| left.event.id.cmp(&right.event.id))
        });
        let matches = matches
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(index, (event, score))| SemanticEventMatch {
                event,
                score,
                rank: index + 1,
            })
            .collect();
        Ok(SemanticEventSearchResult {
            embedding_model: self.embedder.model_id().to_owned(),
            embedding_revision: self.embedder.revision().to_owned(),
            candidates_scanned,
            matches,
        })
    }
}

pub struct ToolEmbeddingIndex {
    embedder: std::sync::Arc<dyn Embedder>,
    store: SharedEventStore,
    state: std::sync::RwLock<IndexState>,
}

impl ToolEmbeddingIndex {
    pub fn new(embedder: std::sync::Arc<dyn Embedder>, store: SharedEventStore) -> Self {
        Self {
            embedder,
            store,
            state: std::sync::RwLock::new(IndexState::default()),
        }
    }

    pub fn model_id(&self) -> &str {
        self.embedder.model_id()
    }

    pub fn revision(&self) -> &str {
        self.embedder.revision()
    }

    pub fn ensure_catalog(&self, generation: &str, definitions: &[ToolDefinition]) -> Result<()> {
        if self
            .state
            .read()
            .map_err(|_| anyhow::anyhow!("tool embedding index lock poisoned"))?
            .catalogs
            .contains_key(generation)
        {
            return Ok(());
        }
        let (stored, reusable) = {
            let store = self
                .store
                .lock()
                .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?;
            (
                store.load_tool_embeddings(
                    generation,
                    self.embedder.model_id(),
                    self.embedder.revision(),
                    self.embedder.dimensions(),
                )?,
                store.load_reusable_tool_embeddings(
                    self.embedder.model_id(),
                    self.embedder.revision(),
                    self.embedder.dimensions(),
                )?,
            )
        };
        let stored = stored
            .into_iter()
            .map(|record| (record.tool_name.clone(), record))
            .collect::<std::collections::HashMap<_, _>>();
        let reusable = reusable
            .into_iter()
            .map(|record| {
                (
                    (
                        record.tool_name.clone(),
                        record.retrieval_text_sha256.clone(),
                    ),
                    record.vector,
                )
            })
            .collect::<std::collections::HashMap<_, _>>();
        let documents = definitions
            .iter()
            .map(|definition| {
                let text = canonical_tool_text(definition);
                let hash = format!("{:x}", Sha256::digest(text.as_bytes()));
                (definition.name.clone(), text, hash)
            })
            .collect::<Vec<_>>();
        let missing = documents
            .iter()
            .filter(|(name, _, hash)| {
                stored
                    .get(name)
                    .is_none_or(|record| record.retrieval_text_sha256 != *hash)
                    && !reusable.contains_key(&(name.clone(), hash.clone()))
            })
            .map(|(_, text, _)| text.clone())
            .collect::<Vec<_>>();
        let generated = if missing.is_empty() {
            Vec::new()
        } else {
            self.embedder.embed(&missing)?
        };
        let mut generated = generated.into_iter();
        let mut tools = Vec::with_capacity(documents.len());
        for (name, _, hash) in documents {
            let (vector, needs_save) = if let Some(record) = stored
                .get(&name)
                .filter(|record| record.retrieval_text_sha256 == hash)
            {
                (record.vector.clone(), false)
            } else if let Some(vector) = reusable.get(&(name.clone(), hash.clone())) {
                (vector.clone(), true)
            } else {
                let vector = generated
                    .next()
                    .context("embedding model omitted a tool vector")?;
                self.store
                    .lock()
                    .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
                    .save_tool_embedding(
                        generation,
                        &name,
                        &hash,
                        self.embedder.model_id(),
                        self.embedder.revision(),
                        self.embedder.dimensions(),
                        &vector,
                    )?;
                (vector, false)
            };
            if needs_save {
                self.store
                    .lock()
                    .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
                    .save_tool_embedding(
                        generation,
                        &name,
                        &hash,
                        self.embedder.model_id(),
                        self.embedder.revision(),
                        self.embedder.dimensions(),
                        &vector,
                    )?;
            }
            tools.push(IndexedTool { name, vector });
        }
        ensure!(
            generated.next().is_none(),
            "embedding model returned too many tool vectors"
        );
        tools.sort_by(|left, right| left.name.cmp(&right.name));
        self.state
            .write()
            .map_err(|_| anyhow::anyhow!("tool embedding index lock poisoned"))?
            .catalogs
            .insert(generation.to_owned(), tools);
        Ok(())
    }

    pub fn search(
        &self,
        generation: &str,
        definitions: &[ToolDefinition],
        query: &str,
        limit: usize,
        minimum_similarity: f32,
    ) -> Result<Vec<SemanticToolMatch>> {
        self.ensure_catalog(generation, definitions)?;
        let text = format!("{EMBEDDING_QUERY_PREFIX}{query}");
        let query = self
            .embedder
            .embed(&[text])?
            .into_iter()
            .next()
            .context("embedding model omitted the query vector")?;
        let state = self
            .state
            .read()
            .map_err(|_| anyhow::anyhow!("tool embedding index lock poisoned"))?;
        let tools = state
            .catalogs
            .get(generation)
            .context("tool embedding catalog is unavailable")?;
        let mut matches = tools
            .iter()
            .filter_map(|tool| {
                let score = normalized_dot(&query, &tool.vector).ok()?;
                (score >= minimum_similarity).then_some((tool.name.clone(), score))
            })
            .collect::<Vec<_>>();
        matches.sort_by(|(left_name, left_score), (right_name, right_score)| {
            right_score
                .total_cmp(left_score)
                .then_with(|| left_name.cmp(right_name))
        });
        Ok(matches
            .into_iter()
            .take(limit)
            .enumerate()
            .map(|(index, (tool, score))| SemanticToolMatch {
                tool,
                score,
                rank: index + 1,
            })
            .collect())
    }

    pub fn used_tools(&self, correlation_id: Uuid) -> Result<Vec<String>> {
        self.store
            .lock()
            .map_err(|_| anyhow::anyhow!("event store lock poisoned"))?
            .called_tools_in_correlation(correlation_id, USED_TOOL_LIMIT)
    }
}

pub fn canonical_tool_text(definition: &ToolDefinition) -> String {
    let mut lines = vec![
        format!("Tool: {}", definition.name),
        format!("Description: {}", definition.description.trim()),
    ];
    if let Some(properties) = definition
        .input_schema
        .get("properties")
        .and_then(serde_json::Value::as_object)
        && !properties.is_empty()
    {
        lines.push("Inputs:".into());
        append_property_documents("", properties, &mut lines);
    }
    lines.join("\n")
}

fn append_property_documents(
    prefix: &str,
    properties: &serde_json::Map<String, serde_json::Value>,
    lines: &mut Vec<String>,
) {
    let mut names = properties.keys().collect::<Vec<_>>();
    names.sort();
    for name in names {
        let path = if prefix.is_empty() {
            name.to_string()
        } else {
            format!("{prefix}.{name}")
        };
        let property = &properties[name];
        let description = property
            .get("description")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim();
        lines.push(if description.is_empty() {
            path.clone()
        } else {
            format!("{path}: {description}")
        });
        if let Some(nested) = property
            .get("properties")
            .and_then(serde_json::Value::as_object)
        {
            append_property_documents(&path, nested, lines);
        }
        if let Some(nested) = property
            .pointer("/items/properties")
            .and_then(serde_json::Value::as_object)
        {
            append_property_documents(&format!("{path}[]"), nested, lines);
        }
    }
}

const EVENT_TOOL_QUERY_BYTES: usize = 16 * 1024;
const EVENT_TOOL_QUERY_NODES: usize = 2_048;
const EVENT_TOOL_QUERY_PATH_BYTES: usize = 512;

pub fn canonical_event_text(event: &Event) -> String {
    event_tool_query(event, &[])
}

pub fn event_tool_query(event: &Event, compiled_context: &[String]) -> String {
    let mut output = String::with_capacity(EVENT_TOOL_QUERY_BYTES);
    append_bounded_line(&mut output, "Event type", &event.event_type);
    append_bounded_line(&mut output, "Event source", &event.source);
    let mut remaining_nodes = EVENT_TOOL_QUERY_NODES;
    collect_scalar_text("payload", &event.payload, &mut output, &mut remaining_nodes);
    for (index, value) in compiled_context.iter().enumerate() {
        if output.len() == EVENT_TOOL_QUERY_BYTES {
            break;
        }
        append_bounded_line(
            &mut output,
            &bounded_path("context", &index.to_string()),
            value,
        );
    }
    output
}

fn append_bounded_line(output: &mut String, path: &str, value: &str) {
    if output.len() == EVENT_TOOL_QUERY_BYTES {
        return;
    }
    if !output.is_empty() {
        output.push('\n');
    }
    for part in [path, ": ", value] {
        let remaining = EVENT_TOOL_QUERY_BYTES - output.len();
        if remaining == 0 {
            break;
        }
        let end = part.floor_char_boundary(remaining.min(part.len()));
        output.push_str(&part[..end]);
    }
}

fn bounded_path(parent: &str, child: &str) -> String {
    let mut path =
        String::with_capacity(EVENT_TOOL_QUERY_PATH_BYTES.min(parent.len() + child.len() + 1));
    for part in [parent, ".", child] {
        let remaining = EVENT_TOOL_QUERY_PATH_BYTES - path.len();
        if remaining == 0 {
            break;
        }
        let end = part.floor_char_boundary(remaining.min(part.len()));
        path.push_str(&part[..end]);
    }
    path
}

fn collect_scalar_text(
    path: &str,
    value: &serde_json::Value,
    output: &mut String,
    remaining_nodes: &mut usize,
) {
    if output.len() == EVENT_TOOL_QUERY_BYTES || *remaining_nodes == 0 {
        return;
    }
    *remaining_nodes -= 1;
    match value {
        serde_json::Value::String(value) if !value.is_empty() => {
            if Uuid::parse_str(value).is_err() {
                append_bounded_line(output, path, value);
            }
        }
        serde_json::Value::Number(value) => append_bounded_line(output, path, &value.to_string()),
        serde_json::Value::Bool(value) => {
            append_bounded_line(output, path, if *value { "true" } else { "false" });
        }
        serde_json::Value::Array(values) => {
            for (index, value) in values.iter().enumerate() {
                if output.len() == EVENT_TOOL_QUERY_BYTES || *remaining_nodes == 0 {
                    break;
                }
                collect_scalar_text(
                    &bounded_path(path, &index.to_string()),
                    value,
                    output,
                    remaining_nodes,
                );
            }
        }
        serde_json::Value::Object(values) => {
            for key in bounded_sorted_object_keys(values, *remaining_nodes) {
                if output.len() == EVENT_TOOL_QUERY_BYTES || *remaining_nodes == 0 {
                    break;
                }
                collect_scalar_text(
                    &bounded_path(path, key),
                    &values[key],
                    output,
                    remaining_nodes,
                );
            }
        }
        _ => {}
    }
}

fn bounded_sorted_object_keys(
    values: &serde_json::Map<String, serde_json::Value>,
    limit: usize,
) -> Vec<&String> {
    let mut keys = values.keys().take(limit).collect::<Vec<_>>();
    keys.sort();
    keys
}

pub fn normalized_dot(left: &[f32], right: &[f32]) -> Result<f32> {
    ensure!(
        left.len() == right.len(),
        "embedding dimensions do not match"
    );
    Ok(left
        .iter()
        .zip(right)
        .map(|(left, right)| left * right)
        .sum())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_corrupt_model_files_fail_clearly() {
        let directory = tempfile::tempdir().unwrap();
        let missing = validate_model_dir(directory.path())
            .unwrap_err()
            .to_string();
        assert!(missing.contains("run 'habibi embeddings install'"));
        for file in MODEL_FILES {
            fs::write(
                directory.path().join(file.name),
                vec![0; file.size.min(8) as usize],
            )
            .unwrap();
        }
        let corrupt = validate_model_dir(directory.path())
            .unwrap_err()
            .to_string();
        assert!(corrupt.contains("invalid size") || corrupt.contains("SHA-256"));
    }

    #[test]
    fn checked_in_manifest_matches_authoritative_constants() {
        let manifest: serde_json::Value =
            serde_json::from_str(include_str!("../models/bge-small-en-v1.5-onnx-q.json")).unwrap();
        assert_eq!(manifest["model"], EMBEDDING_MODEL_ID);
        assert_eq!(manifest["repository"], EMBEDDING_REPOSITORY);
        assert_eq!(manifest["revision"], EMBEDDING_REVISION);
        assert_eq!(manifest["dimensions"], EMBEDDING_DIMENSIONS);
        assert_eq!(manifest["artifact_license"], "Apache-2.0");
        assert_eq!(manifest["upstream_model_license"], "MIT");
        let files = manifest["files"].as_array().unwrap();
        assert_eq!(files.len(), MODEL_FILES.len());
        for expected in MODEL_FILES {
            let actual = files
                .iter()
                .find(|file| file["name"] == expected.name)
                .unwrap();
            assert_eq!(actual["size"], expected.size);
            assert_eq!(actual["sha256"], expected.sha256);
        }
    }

    #[test]
    fn pinned_manifest_has_unique_files_and_expected_model() {
        let names = MODEL_FILES
            .iter()
            .map(|file| file.name)
            .collect::<std::collections::HashSet<_>>();
        assert_eq!(names.len(), MODEL_FILES.len());
        assert_eq!(EMBEDDING_DIMENSIONS, 384);
        assert_eq!(MODEL_FILES[0].size, 66_465_124);
        assert_eq!(MODEL_FILES[0].sha256.len(), 64);
    }

    struct FakeEmbedder {
        calls: std::sync::atomic::AtomicUsize,
    }

    impl FakeEmbedder {
        fn new() -> Self {
            Self {
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    impl Embedder for FakeEmbedder {
        fn model_id(&self) -> &str {
            "fake"
        }
        fn revision(&self) -> &str {
            "1"
        }
        fn dimensions(&self) -> usize {
            2
        }
        fn embed(&self, texts: &[String]) -> Result<Vec<Vec<f32>>> {
            self.calls
                .fetch_add(texts.len(), std::sync::atomic::Ordering::Relaxed);
            Ok(texts
                .iter()
                .map(|text| {
                    if text.contains("file") || text.contains("workspace") {
                        vec![1.0, 0.0]
                    } else if text.contains("process") {
                        vec![0.0, 1.0]
                    } else {
                        vec![0.70710677, 0.70710677]
                    }
                })
                .collect())
        }
    }

    fn definition(name: &str, description: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.into(),
            description: description.into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "z": { "type": "string", "description": "Last" },
                    "a": { "type": "string", "description": "First" }
                }
            }),
        }
    }

    #[test]
    fn canonical_tool_text_is_stable_and_omits_schema_syntax() {
        let text = canonical_tool_text(&definition("workspace.read", "Read a file."));
        assert_eq!(
            text,
            "Tool: workspace.read\nDescription: Read a file.\nInputs:\na: First\nz: Last"
        );
        assert!(!text.contains("type"));
    }

    #[test]
    fn event_tool_query_is_deterministic_bounded_and_omits_uuid_noise() {
        let event = Event::new(
            "example.created",
            "extension:example",
            Uuid::now_v7(),
            None,
            serde_json::json!({
                "id": Uuid::now_v7().to_string(),
                "content": "find a file",
                "large": "x".repeat(20_000),
            }),
        );
        let left = event_tool_query(&event, &[]);
        let right = event_tool_query(&event, &[]);
        assert_eq!(left, right);
        assert!(left.len() <= 16 * 1024);
        assert!(left.contains("find a file"));
        assert!(!left.contains(&event.payload["id"].as_str().unwrap()[..8]));
    }

    #[test]
    fn event_tool_query_stops_after_the_bounded_node_budget() {
        let event = Event::new(
            "example.created",
            "extension:example",
            Uuid::now_v7(),
            None,
            serde_json::json!({ "empty": vec![serde_json::Value::Null; 100_000] }),
        );
        let query = event_tool_query(&event, &["formatted extension context".into()]);
        assert!(query.len() <= EVENT_TOOL_QUERY_BYTES);
        assert!(query.starts_with("Event type: example.created\nEvent source: extension:example"));
        assert!(query.contains("formatted extension context"));
    }

    #[test]
    fn event_tool_query_limits_wide_object_keys_before_collection() {
        let wide = (0..100_000)
            .map(|index| (format!("key-{index:06}"), serde_json::Value::Null))
            .collect::<serde_json::Map<_, _>>();
        assert_eq!(
            bounded_sorted_object_keys(&wide, EVENT_TOOL_QUERY_NODES).len(),
            EVENT_TOOL_QUERY_NODES
        );
        let event = Event::new(
            "example.created",
            "extension:example",
            Uuid::now_v7(),
            None,
            serde_json::Value::Object(wide),
        );
        let query = event_tool_query(&event, &["formatted extension context".into()]);
        assert!(query.len() <= EVENT_TOOL_QUERY_BYTES);
        assert!(query.starts_with("Event type: example.created\nEvent source: extension:example"));
        assert!(query.contains("formatted extension context"));
    }

    #[test]
    fn semantic_event_search_is_bounded_before_sequence_and_persistent() {
        let store = crate::store::EventStore::open(":memory:").unwrap().shared();
        let file = Event::new(
            "workspace.file.observed",
            "test",
            Uuid::now_v7(),
            None,
            serde_json::json!({ "content": "important file notes" }),
        );
        let process = Event::new(
            "process.completed",
            "test",
            file.correlation_id,
            None,
            serde_json::json!({ "content": "process output" }),
        );
        let trigger = Event::new(
            "test.query",
            "test",
            file.correlation_id,
            None,
            serde_json::json!({ "content": "workspace question" }),
        );
        let before_sequence = {
            let locked = store.lock().unwrap();
            locked.append(&file).unwrap();
            locked.append(&process).unwrap();
            locked.append(&trigger).unwrap()
        };
        let embedder = std::sync::Arc::new(FakeEmbedder::new());
        let index = EventEmbeddingIndex::new(embedder.clone(), store);
        let first = index
            .search("workspace file", before_sequence, 20, 0.60)
            .unwrap();
        assert_eq!(first.matches.len(), 1);
        assert_eq!(first.matches[0].event.event.id, file.id);
        assert_eq!(first.embedding_model, "fake");
        assert_eq!(first.candidates_scanned, 2);
        assert_eq!(embedder.calls.load(std::sync::atomic::Ordering::Relaxed), 3);
        let second = index
            .search("workspace file", before_sequence, 20, 0.60)
            .unwrap();
        assert_eq!(second.matches[0].event.event.id, file.id);
        assert_eq!(embedder.calls.load(std::sync::atomic::Ordering::Relaxed), 4);
    }

    #[test]
    fn semantic_ranking_applies_threshold_and_exact_name_ties() {
        let store = crate::store::EventStore::open(":memory:").unwrap().shared();
        let embedder = std::sync::Arc::new(FakeEmbedder::new());
        let index = ToolEmbeddingIndex::new(embedder, store);
        let definitions = vec![
            definition("workspace.write", "Write a file"),
            definition("workspace.read", "Read a file"),
            definition("process.run", "Run a process"),
        ];
        let matches = index
            .search("generation", &definitions, "find a file", 50, 0.8)
            .unwrap();
        assert_eq!(
            matches
                .iter()
                .map(|value| value.tool.as_str())
                .collect::<Vec<_>>(),
            ["workspace.read", "workspace.write"]
        );
        assert_eq!(matches[0].rank, 1);
    }

    #[test]
    fn persisted_vectors_are_reused_and_changed_text_is_reembedded() {
        let store = crate::store::EventStore::open(":memory:").unwrap().shared();
        let embedder = std::sync::Arc::new(FakeEmbedder::new());
        let first = ToolEmbeddingIndex::new(embedder.clone(), store.clone());
        first
            .ensure_catalog("generation", &[definition("workspace.read", "Read a file")])
            .unwrap();
        assert_eq!(embedder.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        let second = ToolEmbeddingIndex::new(embedder.clone(), store);
        second
            .ensure_catalog("generation", &[definition("workspace.read", "Read a file")])
            .unwrap();
        assert_eq!(embedder.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        second
            .ensure_catalog(
                "new-generation",
                &[definition("workspace.read", "Read a file")],
            )
            .unwrap();
        assert_eq!(embedder.calls.load(std::sync::atomic::Ordering::Relaxed), 1);
        second
            .ensure_catalog(
                "changed-generation",
                &[definition("workspace.read", "Read files safely")],
            )
            .unwrap();
        assert_eq!(embedder.calls.load(std::sync::atomic::Ordering::Relaxed), 2);
    }

    #[test]
    fn normalized_dot_is_deterministic() {
        assert_eq!(normalized_dot(&[1.0, 0.0], &[0.5, 0.5]).unwrap(), 0.5);
        assert!(normalized_dot(&[1.0], &[1.0, 0.0]).is_err());
    }
}
