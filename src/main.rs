use std::env;

#[cfg(feature = "openapi")]
use std::fs;
#[cfg(feature = "openapi")]
use std::path::PathBuf;

use axum::{Router, middleware};
use tokio::net::TcpListener;

use anyhow::anyhow;

use sayna::{ServerConfig, init, middleware::auth::auth_middleware, routes, state::AppState};

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
            #[cfg(feature = "openapi")]
            "openapi" => {
                // Parse openapi command arguments
                let mut format = "yaml".to_string();
                let mut output: Option<PathBuf> = None;

                while let Some(arg) = args.next() {
                    match arg.as_str() {
                        "-f" | "--format" => {
                            format = args.next()
                                .ok_or_else(|| anyhow!("--format requires a value (yaml or json)"))?;
                            if format != "yaml" && format != "json" {
                                anyhow::bail!("Invalid format '{}'. Must be 'yaml' or 'json'", format);
                            }
                        }
                        "-o" | "--output" => {
                            let path = args.next()
                                .ok_or_else(|| anyhow!("--output requires a file path"))?;
                            output = Some(PathBuf::from(path));
                        }
                        other => {
                            anyhow::bail!("Unknown option '{}'. Use --format (yaml|json) or --output <file>", other);
                        }
                    }
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
                    fs::write(&output_path, &spec_content)
                        .map_err(|e| anyhow!("Failed to write to {}: {}", output_path.display(), e))?;
                    println!("OpenAPI spec written to {}", output_path.display());
                } else {
                    println!("{}", spec_content);
                }

                return Ok(());
            }
            other => {
                #[cfg(feature = "openapi")]
                {
                    anyhow::bail!("Unknown command '{other}'. Supported commands: init, openapi");
                }
                #[cfg(not(feature = "openapi"))]
                {
                    anyhow::bail!("Unknown command '{other}'. Supported commands: init");
                }
            }
        }
    }

    // Load configuration
    let config = ServerConfig::from_env().map_err(|e| anyhow!(e.to_string()))?;
    let address = config.address();
    println!("Starting server on {address}");

    // Create application state
    let app_state = AppState::new(config).await;

    // Create protected API routes with authentication middleware
    let protected_routes = routes::api::create_api_router().layer(middleware::from_fn_with_state(
        app_state.clone(),
        auth_middleware,
    ));

    // Create WebSocket routes (no auth for now, see action plan task 8)
    let ws_routes = routes::ws::create_ws_router();

    // Create public health check route (no auth)
    let public_routes =
        Router::new().route("/", axum::routing::get(sayna::handlers::api::health_check));

    // Combine all routes: public + protected + websocket
    let app = public_routes
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
