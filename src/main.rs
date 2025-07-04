use tokio::net::TcpListener;

use sayna::{ServerConfig, routes, state::AppState};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();

    // Load configuration
    let config = ServerConfig::from_env()?;
    let address = config.address();

    println!("Starting server on {address}");

    // Create application state
    let app_state = AppState::new(config);

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
