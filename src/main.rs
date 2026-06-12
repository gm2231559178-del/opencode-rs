mod acp;
mod cli;
mod config;
mod llm;
mod log;
mod mdns;
mod server;
mod session;
mod session_store;
mod theme;
mod tools;
mod tui;

use anyhow::Result;

#[tokio::main]
async fn main() -> Result<()> {
    let args = cli::parse();
    log::init(&args.log_level);

    let mut cfg = config::load_config()?;
    cli::merge_cli_config(&mut cfg, &args);

    let store = session_store::SessionStore::new().ok();

    match &args.command {
        Some(cli::Commands::Start { directory }) => {
            if let Some(dir) = directory {
                std::env::set_current_dir(dir)?;
            }
            let session = session::Session::new(cfg)?;
            let mut app = tui::TuiApp::new(session, store);
            app.run().await?;
        }
        Some(cli::Commands::Run { prompt }) => {
            let input = prompt.join(" ");
            let mut session = session::Session::new(cfg)?;
            session.prompt(&input).await?;
            println!("{}", session.last_response.trim());
        }
        Some(cli::Commands::Config { action }) => match action {
            cli::ConfigAction::Show => {
                println!("{}", serde_json::to_string_pretty(&cfg)?);
            }
            cli::ConfigAction::Set { key, value } => {
                println!("Setting {} = {} (not yet implemented)", key, value);
            }
        },
        Some(cli::Commands::Serve { port }) => {
            server::run_server(cfg, store, *port).await;
        }
        Some(cli::Commands::Acp) => {
            let server = acp::AcpServer::new(cfg, store);
            server.run().await?;
        }
        Some(cli::Commands::Version) => {
            println!("opencode-rs v{}", env!("CARGO_PKG_VERSION"));
        }
        None => {
            if let Some(prompt) = &args.prompt {
                let mut session = session::Session::new(cfg)?;
                session.prompt(prompt).await?;
                println!("{}", session.last_response.trim());
            } else {
                let session = session::Session::new(cfg)?;
                let mut app = tui::TuiApp::new(session, store);
                app.run().await?;
            }
        }
    }

    Ok(())
}
