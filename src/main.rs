use std::path::PathBuf;

#[cfg(feature = "openapi")]
use std::fs;

use axum::{Router, middleware};
use clap::{Parser, Subcommand};
use tokio::net::TcpListener;

use anyhow::anyhow;

use sayna::{ServerConfig, init, middleware::auth::auth_middleware, routes, state::AppState};

/// Sayna - Real-time voice processing server
#[derive(Parser, Debug)]
#[command(name = "sayna")]
#[command(version, about, long_about = None)]
struct Cli {
    /// Path to configuration file (YAML)
    #[arg(short = 'c', long = "config", value_name = "FILE")]
    config: Option<PathBuf>,

    /// Subcommand to run
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Initialize Sayna by downloading required models.
    ///
    /// Downloads models for enabled features:
    ///
    /// - stt-vad: Turn detection model, tokenizer, and Silero-VAD model for
    ///   voice activity detection with integrated turn detection
    ///
    /// Requires CACHE_PATH environment variable to be set.
    Init,

    /// Generate OpenAPI specification
    #[cfg(feature = "openapi")]
    Openapi {
        /// Output format (yaml or json)
        #[arg(short = 'f', long = "format", default_value = "yaml")]
        format: String,

        /// Output file path (prints to stdout if not specified)
        #[arg(short = 'o', long = "output")]
        output: Option<PathBuf>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Load .env file if it exists (must be done before config loading)
    let _ = dotenvy::dotenv();

    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Initialize crypto provider for TLS connections
    // This must be done before any TLS connections are attempted
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| anyhow!("Failed to install default crypto provider"))?;

    // Parse CLI arguments
    let cli = Cli::parse();

    // Handle subcommands
    if let Some(command) = cli.command {
        match command {
            Commands::Init => {
                init::run().await?;
                return Ok(());
            }
            #[cfg(feature = "openapi")]
            Commands::Openapi { format, output } => {
                // Validate format
                if format != "yaml" && format != "json" {
                    anyhow::bail!("Invalid format '{}'. Must be 'yaml' or 'json'", format);
                }

                // Generate the spec in the requested format
                let spec_content = match format.as_str() {
                    "yaml" => sayna::docs::openapi::spec_yaml()
                        .map_err(|e| anyhow!("Failed to generate OpenAPI YAML: {}", e))?,
                    "json" => sayna::docs::openapi::spec_json()
                        .map_err(|e| anyhow!("Failed to generate OpenAPI JSON: {}", e))?,
                    _ => unreachable!(),
                };

                // Write to file or stdout
                if let Some(output_path) = output {
                    fs::write(&output_path, &spec_content).map_err(|e| {
                        anyhow!("Failed to write to {}: {}", output_path.display(), e)
                    })?;
                    println!("OpenAPI spec written to {}", output_path.display());
                } else {
                    println!("{}", spec_content);
                }

                return Ok(());
            }
        }
    }

    // Load configuration from file or environment
    let config = if let Some(config_path) = cli.config {
        println!("Loading configuration from {}", config_path.display());
        ServerConfig::from_file(&config_path).map_err(|e| anyhow!(e.to_string()))?
    } else {
        ServerConfig::from_env().map_err(|e| anyhow!(e.to_string()))?
    };

    let address = config.address();
    println!("Starting server on {address}");

    // Create application state
    let app_state = AppState::new(config).await;

    // Create protected API routes with authentication middleware
    let protected_routes = routes::api::create_api_router().layer(middleware::from_fn_with_state(
        app_state.clone(),
        auth_middleware,
    ));

    // Create WebSocket routes with auth middleware for tenant isolation
    // When auth is enabled: requires valid auth token
    // When auth is disabled: inserts empty Auth context for room name handling
    let ws_routes = routes::ws::create_ws_router().layer(middleware::from_fn_with_state(
        app_state.clone(),
        auth_middleware,
    ));

    // Create webhook routes (no auth - uses LiveKit signature verification)
    let webhook_routes = routes::webhooks::create_webhook_router();

    // Create public health check route (no auth)
    let public_routes =
        Router::new().route("/", axum::routing::get(sayna::handlers::api::health_check));

    // Combine all routes: public + webhook + protected + websocket
    let app = public_routes
        .merge(webhook_routes)
        .merge(protected_routes)
        .merge(ws_routes)
        .with_state(app_state);

    // Create listener
    let listener = TcpListener::bind(&address).await?;

    println!("Server listening on {address}");

    // Start server
    axum::serve(listener, app).await?;

    Ok(())
}
