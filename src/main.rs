mod auth;
mod event;
mod model;
mod reactor;
mod store;

use std::io::{self, Write};

use anyhow::{Context, Result, bail};
use auth::CredentialStore;
use model::{ModelClient, ModelConfig};
use reactor::Reactor;
use store::EventStore;

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
    let context_message_limit = std::env::var("HABIBI_CONTEXT_MESSAGES")
        .unwrap_or_else(|_| "40".to_owned())
        .parse::<usize>()
        .context("HABIBI_CONTEXT_MESSAGES must be a positive integer")?;
    if context_message_limit == 0 {
        anyhow::bail!("HABIBI_CONTEXT_MESSAGES must be greater than zero");
    }

    let model = ModelClient::new(ModelConfig::from_env()?)?;
    let store = EventStore::open(&database_path)?;
    let reactor = Reactor::new(store, model, context_message_limit);
    reactor.record_runtime_started()?;

    println!("Habibi — one continuous conversation");
    println!("Event store: {database_path}");
    println!("Type /quit to leave.\n");

    loop {
        print!("> ");
        io::stdout().flush()?;

        let mut input = String::new();
        if io::stdin().read_line(&mut input)? == 0 {
            println!();
            break;
        }

        let input = input.trim();
        if input == "/quit" || input == "/exit" {
            break;
        }
        if input.is_empty() {
            continue;
        }

        match reactor.receive_user_message(input.to_owned()).await {
            Ok(Some(response)) => println!("\n{response}\n"),
            Ok(None) => println!("\n[no response]\n"),
            Err(error) => eprintln!("\nModel error: {error:#}\n"),
        }
    }

    Ok(())
}
