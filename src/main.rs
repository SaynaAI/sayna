use std::env;

use tokio::net::TcpListener;

use anyhow::anyhow;

use sayna::{ServerConfig, init, routes, state::AppState};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Initialize crypto provider for TLS connections
    // This must be done before any TLS connections are attempted
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow!("Failed to install default crypto provider"))?;

    // Handle CLI commands
    let mut args = env::args();
    let _ = args.next();
    if let Some(command) = args.next() {
        match command.as_str() {
            "init" => {
                if let Some(extra) = args.next() {
                    anyhow::bail!("Unexpected argument '{extra}' after 'init'");
                }
                init::run().await?;
                return Ok(());
            }
            other => {
                anyhow::bail!("Unknown command '{other}'. Supported commands: init");
            }
        }
    }

    // Load configuration
    let config = ServerConfig::from_env().map_err(|e| anyhow!(e.to_string()))?;
    let address = config.address();
    println!("Starting server on {address}");

    // Create application state
    let app_state = AppState::new(config).await;

    // Create router with both API and WebSocket routes
    let app = routes::api::create_api_router()
        .merge(routes::ws::create_ws_router())
        .with_state(app_state);

    // Create listener
    let listener = TcpListener::bind(&address).await?;

    println!("Server listening on {address}");

    // Start server
    axum::serve(listener, app).await?;

    Ok(())
}
