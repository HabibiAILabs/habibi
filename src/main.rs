mod auth;
mod catalog;
mod event;
mod extension;
mod installer;
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
use installer::{ExtensionInstaller, SourceOptions};
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
    let extensions_path = PathBuf::from(
        std::env::var("HABIBI_EXTENSIONS_DIR").unwrap_or_else(|_| "extensions".to_owned()),
    );
    if arguments.first().map(String::as_str) == Some("install") {
        let (source, options) = install_arguments(&arguments[1..])?;
        let installed = ExtensionInstaller::new(extensions_path).install(&source, options)?;
        println!("Installed {} {}", installed.name, installed.version);
        println!("Extension ID: {}", installed.id);
        notify_extension_reload(&installed.id).await;
        return Ok(());
    }
    if arguments.first().map(String::as_str) == Some("update") {
        if arguments.len() != 2 {
            bail!("usage: habibi update <extension-id>");
        }
        let installed = ExtensionInstaller::new(extensions_path).update(&arguments[1])?;
        println!("Updated {} to {}", installed.name, installed.version);
        notify_extension_reload(&installed.id).await;
        return Ok(());
    }
    if arguments.first().map(String::as_str) == Some("rollback") {
        if arguments.len() != 2 {
            bail!("usage: habibi rollback <extension-id>");
        }
        let installed = ExtensionInstaller::new(extensions_path).rollback(&arguments[1])?;
        println!("Rolled back {} to {}", installed.name, installed.version);
        notify_extension_reload(&installed.id).await;
        return Ok(());
    }
    if !arguments.is_empty() {
        bail!(
            "unknown command '{}'; supported commands: login, install, update, rollback",
            arguments[0]
        );
    }

    let database_path = std::env::var("HABIBI_DB").unwrap_or_else(|_| "habibi.db".to_owned());
    let bind_address = std::env::var("HABIBI_BIND").unwrap_or_else(|_| "127.0.0.1:8787".to_owned());
    let extension_bind_address =
        std::env::var("HABIBI_EXTENSION_BIND").unwrap_or_else(|_| "127.0.0.1:8788".to_owned());
    let core_origin =
        std::env::var("HABIBI_CORE_ORIGIN").unwrap_or_else(|_| format!("http://{bind_address}"));
    let extension_origin = std::env::var("HABIBI_EXTENSION_ORIGIN")
        .unwrap_or_else(|_| format!("http://{extension_bind_address}"));
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
    let state = WebState {
        extensions: extensions.clone(),
        reactor,
        store,
        extensions_dir: extensions_path.clone(),
        core_origin,
        extension_origin,
        reaction_lock: Arc::new(tokio::sync::Mutex::new(())),
    };
    let app = web::router(state.clone());
    let extension_app = web::extension_router(state);

    let listener = tokio::net::TcpListener::bind(&bind_address)
        .await
        .with_context(|| format!("failed to bind Habibi web server to {bind_address}"))?;
    let extension_listener = tokio::net::TcpListener::bind(&extension_bind_address)
        .await
        .with_context(|| {
            format!("failed to bind Habibi extension web server to {extension_bind_address}")
        })?;
    println!("Habibi — one continuous event stream");
    println!("Event store: {database_path}");
    println!("Extensions: {}", extensions_path.display());
    println!("Web: http://{bind_address}");
    println!("Extension web: http://{extension_bind_address}");

    tokio::select! {
        result = axum::serve(listener, app) => result?,
        result = axum::serve(extension_listener, extension_app) => result?,
        _ = shutdown_signal() => {}
    }
    Ok(())
}

async fn notify_extension_reload(extension_id: &str) {
    let core_origin = std::env::var("HABIBI_CORE_ORIGIN").unwrap_or_else(|_| {
        let bind_address =
            std::env::var("HABIBI_BIND").unwrap_or_else(|_| "127.0.0.1:8787".to_owned());
        format!("http://{bind_address}")
    });
    let url = format!(
        "{}/api/extensions/{}/reload",
        core_origin.trim_end_matches('/'),
        url::form_urlencoded::byte_serialize(extension_id.as_bytes()).collect::<String>()
    );
    let result = reqwest::Client::new()
        .post(url)
        .header("x-habibi-admin-request", "core-ui")
        .send()
        .await;
    match result {
        Ok(response) if response.status().is_success() => {
            println!("Reloaded the running Habibi extension runtime.");
        }
        _ => {
            println!("Habibi is not running; the extension will load on the next start.");
        }
    }
}

fn install_arguments(arguments: &[String]) -> Result<(String, SourceOptions)> {
    let Some(source) = arguments.first() else {
        bail!("usage: habibi install <path-or-git-url> [--ref <ref>] [--subdir <path>]");
    };
    let mut options = SourceOptions::default();
    let mut index = 1;
    while index < arguments.len() {
        let value = arguments
            .get(index + 1)
            .with_context(|| format!("{} requires a value", arguments[index]))?;
        match arguments[index].as_str() {
            "--ref" => options.reference = Some(value.clone()),
            "--subdir" => options.subdir = Some(value.clone()),
            option => bail!("unknown install option '{option}'"),
        }
        index += 2;
    }
    Ok((source.clone(), options))
}

async fn shutdown_signal() {
    let _ = tokio::signal::ctrl_c().await;
}
