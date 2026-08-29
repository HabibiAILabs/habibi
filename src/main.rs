mod auth;
mod catalog;
mod context;
mod event;
mod extension;
mod filesystem;
mod installer;
mod model;
#[cfg(target_os = "linux")]
mod process;
mod reactor;
mod scanner;
mod search;
mod store;
mod studio;
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
use studio::StudioService;
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
        print_scan_summary(&installed);
        notify_extension_reload(&installed.id).await;
        return Ok(());
    }
    if arguments.first().map(String::as_str) == Some("update") {
        if arguments.len() != 2 {
            bail!("usage: habibi update <extension-id>");
        }
        let installed = ExtensionInstaller::new(extensions_path).update(&arguments[1])?;
        println!("Updated {} to {}", installed.name, installed.version);
        print_scan_summary(&installed);
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
    let catalog = CatalogManager::from_env()?;
    let model = ModelClient::new(ModelConfig::from_env()?, catalog)?;
    let store = EventStore::open(&database_path)?.shared();
    let extensions = Arc::new(ExtensionManager::load(&extensions_path, store.clone())?);
    let studio = Arc::new(StudioService::from_env()?);
    let extensions_root = extensions_path.canonicalize()?;
    if studio.root_path().starts_with(&extensions_root)
        || extensions_root.starts_with(studio.root_path())
    {
        bail!("extension drafts and installed extensions must use separate directories");
    }
    let tools = Arc::new(ToolRuntime::new(store.clone(), extensions.clone())?);
    let reactor = Arc::new(Reactor::new(store.clone(), model, tools));
    reactor.record_runtime_started()?;
    let state = WebState {
        extensions: extensions.clone(),
        reactor,
        store,
        extensions_dir: extensions_path.clone(),
        studio,
        local_admin: bind_address
            .parse::<std::net::SocketAddr>()
            .is_ok_and(|address| address.ip().is_loopback()),
        reaction_lock: Arc::new(tokio::sync::Mutex::new(())),
    };
    let app = web::router(state.clone());

    let listener = tokio::net::TcpListener::bind(&bind_address)
        .await
        .with_context(|| format!("failed to bind Habibi web server to {bind_address}"))?;
    println!("Habibi — one continuous event stream");
    println!("Event store: {database_path}");
    println!("Extensions: {}", extensions_path.display());
    println!("Extension drafts: {}", state.studio.root_path().display());
    println!("Web: http://{bind_address}");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    Ok(())
}

fn print_scan_summary(installed: &installer::InstallMetadata) {
    if let Some(scan) = &installed.security_scan {
        println!(
            "Security/privacy scan: passed ({} files, {} warnings)",
            scan.files_scanned, scan.warning_count
        );
        for finding in &scan.findings {
            println!(
                "  {:?}: {} — {}",
                finding.severity, finding.file, finding.message
            );
        }
    }
}

async fn notify_extension_reload(extension_id: &str) {
    let bind_address = std::env::var("HABIBI_BIND").unwrap_or_else(|_| "127.0.0.1:8787".to_owned());
    let core_origin = format!("http://{bind_address}");
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
