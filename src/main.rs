mod auth;
mod catalog;
mod event;
mod extension;
mod model;
mod reactor;
mod store;
mod tool;
mod web;

use std::{path::PathBuf, sync::Arc};

use anyhow::{Context, Result, bail};
use auth::CredentialStore;
use catalog::CatalogManager;
use extension::ExtensionManager;
use model::{ModelClient, ModelConfig};
use reactor::Reactor;
use store::EventStore;
use tool::ToolRuntime;
use web::WebState;

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();

    let arguments = std::env::args().skip(1).collect::<Vec<_>>();
    if arguments.first().map(String::as_str) == Some("login") {
        if arguments.len() > 2
            || arguments
                .get(1)
                .is_some_and(|provider| provider != "openai")
        {
            bail!("usage: habibi login [openai]");
        }
        let client = reqwest::Client::builder()
            .user_agent(concat!("habibi/", env!("CARGO_PKG_VERSION")))
            .build()?;
        CredentialStore::from_env()?.login_openai(&client).await?;
        return Ok(());
    }
    if !arguments.is_empty() {
        bail!(
            "unknown command '{}'; supported command: login",
            arguments[0]
        );
    }

    let database_path = std::env::var("HABIBI_DB").unwrap_or_else(|_| "habibi.db".to_owned());
    let extensions_path = PathBuf::from(
        std::env::var("HABIBI_EXTENSIONS_DIR").unwrap_or_else(|_| "extensions".to_owned()),
    );
    let bind_address = std::env::var("HABIBI_BIND").unwrap_or_else(|_| "127.0.0.1:8787".to_owned());
    let context_message_limit = std::env::var("HABIBI_CONTEXT_MESSAGES")
        .unwrap_or_else(|_| "40".to_owned())
        .parse::<usize>()
        .context("HABIBI_CONTEXT_MESSAGES must be a positive integer")?;
    if context_message_limit == 0 {
        bail!("HABIBI_CONTEXT_MESSAGES must be greater than zero");
    }

    let catalog = CatalogManager::from_env()?;
    let model = ModelClient::new(ModelConfig::from_env()?, catalog)?;
    let store = EventStore::open(&database_path)?.shared();
    let extensions = Arc::new(ExtensionManager::load(&extensions_path, store.clone())?);
    let tools = Arc::new(ToolRuntime::new(store.clone(), extensions.clone())?);
    let reactor = Arc::new(Reactor::new(
        store.clone(),
        model,
        tools,
        context_message_limit,
    ));
    reactor.record_runtime_started()?;
    let app = web::router(WebState {
        extensions: extensions.clone(),
        reactor,
        store,
        reaction_lock: Arc::new(tokio::sync::Mutex::new(())),
    });

    let listener = tokio::net::TcpListener::bind(&bind_address)
        .await
        .with_context(|| format!("failed to bind Habibi web server to {bind_address}"))?;
    println!("Habibi — one continuous event stream");
    println!("Event store: {database_path}");
    println!("Extensions: {}", extensions_path.display());
    println!("Web: http://{bind_address}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
