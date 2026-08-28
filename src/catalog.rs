use std::{
    collections::BTreeMap,
    fs,
    path::{Path, PathBuf},
    sync::{Arc, RwLock},
};

use anyhow::{Context, Result, bail};
use chrono::Utc;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;

const DEFAULT_CATALOG: &str = include_str!("../model-catalog.json");
const DEFAULT_SOURCE_URL: &str = "https://models.dev/api.json";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelPricing {
    pub input_usd_per_million: f64,
    pub output_usd_per_million: f64,
    pub cache_read_usd_per_million: Option<f64>,
    pub cache_write_usd_per_million: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CatalogModel {
    pub provider: String,
    pub id: String,
    pub name: String,
    pub pricing: ModelPricing,
    #[serde(default)]
    pub aliases: Vec<String>,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelCatalog {
    pub version: u32,
    pub source: String,
    pub updated_at: String,
    pub models: Vec<CatalogModel>,
}

#[derive(Clone)]
pub struct CatalogManager {
    path: PathBuf,
    source_url: String,
    catalog: Arc<RwLock<ModelCatalog>>,
}

impl CatalogManager {
    pub fn from_env() -> Result<Self> {
        let path = PathBuf::from(
            std::env::var("HABIBI_MODEL_CATALOG")
                .unwrap_or_else(|_| "model-catalog.json".to_owned()),
        );
        let source_url = std::env::var("HABIBI_MODEL_CATALOG_URL")
            .unwrap_or_else(|_| DEFAULT_SOURCE_URL.to_owned());
        let catalog = load_catalog(&path)?;
        Ok(Self {
            path,
            source_url,
            catalog: Arc::new(RwLock::new(catalog)),
        })
    }

    pub fn snapshot(&self) -> Result<ModelCatalog> {
        self.catalog
            .read()
            .map(|catalog| catalog.clone())
            .map_err(|_| anyhow::anyhow!("model catalog lock poisoned"))
    }

    pub fn lookup(&self, provider: &str, model: &str) -> Result<Option<CatalogModel>> {
        let catalog = self
            .catalog
            .read()
            .map_err(|_| anyhow::anyhow!("model catalog lock poisoned"))?;
        Ok(catalog
            .models
            .iter()
            .find(|entry| {
                entry.provider == provider
                    && (entry.id == model || entry.aliases.iter().any(|alias| alias == model))
            })
            .or_else(|| {
                catalog.models.iter().find(|entry| {
                    entry.id == model || entry.aliases.iter().any(|alias| alias == model)
                })
            })
            .cloned())
    }

    pub async fn refresh(&self, client: &Client) -> Result<ModelCatalog> {
        let response = client
            .get(&self.source_url)
            .send()
            .await
            .context("failed to fetch model catalog")?;
        let status = response.status();
        let value: Value = response
            .json()
            .await
            .context("model catalog source returned invalid JSON")?;
        if !status.is_success() {
            bail!("model catalog source returned {status}");
        }
        let existing = self.snapshot()?;
        let refreshed = merge_models_dev(existing, &value, &self.source_url)?;
        persist_catalog(&self.path, &refreshed)?;
        *self
            .catalog
            .write()
            .map_err(|_| anyhow::anyhow!("model catalog lock poisoned"))? = refreshed.clone();
        Ok(refreshed)
    }
}

fn load_catalog(path: &Path) -> Result<ModelCatalog> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => DEFAULT_CATALOG.to_owned(),
        Err(error) => {
            return Err(error).with_context(|| format!("failed to read {}", path.display()));
        }
    };
    let catalog: ModelCatalog = serde_json::from_str(&contents)
        .with_context(|| format!("invalid model catalog {}", path.display()))?;
    validate_catalog(&catalog)?;
    Ok(catalog)
}

fn validate_catalog(catalog: &ModelCatalog) -> Result<()> {
    if catalog.version != 1 {
        bail!("unsupported model catalog version {}", catalog.version);
    }
    for model in &catalog.models {
        if model.provider.is_empty() || model.id.is_empty() {
            bail!("model catalog entries require provider and id");
        }
        for rate in [
            Some(model.pricing.input_usd_per_million),
            Some(model.pricing.output_usd_per_million),
            model.pricing.cache_read_usd_per_million,
            model.pricing.cache_write_usd_per_million,
        ]
        .into_iter()
        .flatten()
        {
            if !rate.is_finite() || rate < 0.0 {
                bail!("model catalog contains an invalid price for {}", model.id);
            }
        }
    }
    Ok(())
}

fn merge_models_dev(
    existing: ModelCatalog,
    source: &Value,
    source_url: &str,
) -> Result<ModelCatalog> {
    let providers = source
        .as_object()
        .context("model catalog source root must be an object")?;
    let refreshed_at = Utc::now().to_rfc3339();
    let mut models = existing
        .models
        .into_iter()
        .map(|model| ((model.provider.clone(), model.id.clone()), model))
        .collect::<BTreeMap<_, _>>();
    for (provider_id, provider) in providers {
        if provider_id != "openai" {
            continue;
        }
        let Some(provider_models) = provider.get("models").and_then(Value::as_object) else {
            continue;
        };
        for (model_id, model) in provider_models {
            let Some(cost) = model.get("cost") else {
                continue;
            };
            let (Some(input), Some(output)) = (
                cost.get("input").and_then(Value::as_f64),
                cost.get("output").and_then(Value::as_f64),
            ) else {
                continue;
            };
            let key = (provider_id.clone(), model_id.clone());
            let aliases = models
                .get(&key)
                .map(|entry| entry.aliases.clone())
                .unwrap_or_default();
            models.insert(
                key,
                CatalogModel {
                    provider: provider_id.clone(),
                    id: model_id.clone(),
                    name: model
                        .get("name")
                        .and_then(Value::as_str)
                        .unwrap_or(model_id)
                        .to_owned(),
                    pricing: ModelPricing {
                        input_usd_per_million: input,
                        output_usd_per_million: output,
                        cache_read_usd_per_million: cost.get("cache_read").and_then(Value::as_f64),
                        cache_write_usd_per_million: cost
                            .get("cache_write")
                            .and_then(Value::as_f64),
                    },
                    aliases,
                    source: Some(source_url.to_owned()),
                    updated_at: Some(refreshed_at.clone()),
                },
            );
        }
    }
    let codex_ids = models
        .keys()
        .filter(|(provider, _)| provider == "openai-codex")
        .map(|(_, id)| id.clone())
        .collect::<Vec<_>>();
    for id in codex_ids {
        if let Some(openai) = models.get(&("openai".to_owned(), id.clone())).cloned()
            && let Some(codex) = models.get_mut(&("openai-codex".to_owned(), id))
        {
            codex.pricing = openai.pricing;
            codex.source = Some(source_url.to_owned());
            codex.updated_at = Some(refreshed_at.clone());
        }
    }
    let catalog = ModelCatalog {
        version: 1,
        source: source_url.to_owned(),
        updated_at: refreshed_at,
        models: models.into_values().collect(),
    };
    validate_catalog(&catalog)?;
    Ok(catalog)
}

fn persist_catalog(path: &Path, catalog: &ModelCatalog) -> Result<()> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(&temporary, serde_json::to_vec_pretty(catalog)?)?;
    fs::rename(&temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn refresh_merge_preserves_local_aliases() {
        let existing: ModelCatalog = serde_json::from_str(DEFAULT_CATALOG).unwrap();
        let source = serde_json::json!({
            "openai": { "models": { "gpt-test": {
                "name": "GPT Test", "cost": { "input": 1.0, "output": 4.0, "cache_read": 0.2 }
            } } }
        });
        let merged = merge_models_dev(existing, &source, "test").unwrap();
        assert!(merged.models.iter().any(|model| model.id == "gpt-test"));
        assert!(merged.models.iter().any(|model| model.id == "gpt-5.6-luna"));
    }
}
