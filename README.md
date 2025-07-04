# Sayna Server

A simple Axum-based web server with health check endpoint and environment-based configuration, following Axum best practices for code organization.

## Features

- Health check endpoint at `/` that returns `{"status": "OK"}`
- Environment-based configuration with sensible defaults
- Support for `.env` files
- Structured logging with tracing
- Modular architecture following Axum best practices
- Comprehensive error handling
- Integration testing setup

## Project Structure

```
sayna/
├── src/
│   ├── main.rs             # Application entry point
│   ├── lib.rs              # Shared library for core logic
│   ├── config.rs           # Configuration management
│   ├── routes/             # Route definitions
│   │   ├── mod.rs
│   │   └── api.rs          # REST endpoints
│   ├── handlers/           # Request handlers
│   │   ├── mod.rs
│   │   └── api_handlers.rs # API request handlers
│   ├── state/              # Shared application state
│   │   └── mod.rs
│   ├── errors/             # Error types and conversion
│   │   ├── mod.rs
│   │   └── app_error.rs    # Custom error types
│   └── utils/              # Utility functions
│       └── mod.rs
├── tests/                  # Integration tests
│   └── api_tests.rs        # API endpoint tests
├── Cargo.toml
└── README.md
```

## Configuration

The server can be configured using environment variables:

| Variable | Description | Default |
|----------|-------------|---------|
| `HOST` | Server host address | `0.0.0.0` |
| `PORT` | Server port number | `3001` |

## Usage

### Running the server

```bash
cargo run
```

### Using environment variables

You can set environment variables directly:

```bash
HOST=127.0.0.1 PORT=8080 cargo run
```

Or create a `.env` file in the project root:

```
HOST=127.0.0.1
PORT=8080
```

### Testing the health check

Once the server is running, you can test the health check endpoint:

```bash
curl http://localhost:3001/
```

Expected response:
```json
{"status":"OK"}
```

## Development

### Building

```bash
cargo build
```

### Running tests

```bash
cargo test
```

### Running in release mode

```bash
cargo run --release
```

## Architecture

This project follows Axum best practices for code organization:

- **Modular Structure**: Clear separation of concerns with dedicated modules
- **State Management**: Centralized application state using `Arc<AppState>`
- **Error Handling**: Custom error types implementing `IntoResponse`
- **Route Organization**: Logical grouping of routes in dedicated modules
- **Handler Separation**: Request handlers separated from route definitions
- **Testing**: Comprehensive integration tests using Axum's testing utilities 