use std::sync::Arc;

use anyhow::Result;
use rmcp::ServiceExt;

use memorize_mcp::embedding::Embedder;
use memorize_mcp::server::MemorizeServer;
use memorize_mcp::storage::Storage;
use memorize_mcp::transport::ResilientStdioTransport;

struct Args {
    transport: String,
    port: u16,
    db_path: String,
    model_dir: String,
}

fn parse_args() -> Result<Args> {
    let args: Vec<String> = std::env::args().collect();
    let mut transport = "stdio".to_string();
    let mut port: u16 = 8080;
    let mut db_path = "./data".to_string();
    let mut model_dir = "./embedding_model".to_string();

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--transport" => {
                i += 1;
                if i < args.len() {
                    transport = args[i].clone();
                }
            }
            "--port" => {
                i += 1;
                if i < args.len() {
                    port = args[i].parse().map_err(|_| {
                        anyhow::anyhow!(
                            "--port value '{}' is not a valid port number (expected 0-65535)",
                            args[i]
                        )
                    })?;
                }
            }
            "--db-path" => {
                i += 1;
                if i < args.len() {
                    db_path = args[i].clone();
                }
            }
            "--model-dir" => {
                i += 1;
                if i < args.len() {
                    model_dir = args[i].clone();
                }
            }
            "--help" | "-h" => {
                eprintln!(
                    "memorize-mcp\n\n\
                     Options:\n  \
                       --transport <stdio|http>  Transport type (default: stdio)\n  \
                       --port <PORT>             HTTP port (default: 8080)\n  \
                       --db-path <PATH>          Database path (default: ./data)\n  \
                       --model-dir <PATH>        Embedding model directory (default: ./embedding_model)"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown argument: {}", other);
                std::process::exit(1);
            }
        }
        i += 1;
    }

    Ok(Args {
        transport,
        port,
        db_path,
        model_dir,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive(tracing::Level::INFO.into()),
        )
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    let args = parse_args()?;

    tracing::info!("Loading embedding model from {}", args.model_dir);
    let embedder = Arc::new(Embedder::load(
        &format!("{}/model_ort.onnx", args.model_dir),
        &format!("{}/tokenizer.json", args.model_dir),
    )?);
    tracing::info!("Embedding model loaded");

    tracing::info!("Opening storage at {}", args.db_path);
    let storage = Arc::new(Storage::open(&args.db_path).await?);
    tracing::info!("Storage ready");

    let server = MemorizeServer::new(storage, embedder);

    match args.transport.as_str() {
        "stdio" => {
            tracing::info!("Starting stdio transport");
            let transport = ResilientStdioTransport::new();
            let service = server.serve(transport).await?;
            match service.waiting().await {
                Ok(reason) => {
                    tracing::info!("Client disconnected: {:?}", reason);
                }
                Err(e) => {
                    // Client disconnect or malformed message is not a fatal error
                    // for stdio — the pipe is gone, just log and exit gracefully.
                    tracing::warn!("Stdio transport closed: {}", e);
                }
            }
        }
        "http" => {
            use rmcp::transport::streamable_http_server::{
                session::local::LocalSessionManager, StreamableHttpServerConfig,
                StreamableHttpService,
            };

            let ct = tokio_util::sync::CancellationToken::new();
            let bind_addr = format!("0.0.0.0:{}", args.port);
            tracing::info!("Starting HTTP transport on {}", bind_addr);

            let service = StreamableHttpService::new(
                move || Ok(server.clone()),
                LocalSessionManager::default().into(),
                StreamableHttpServerConfig {
                    cancellation_token: ct.child_token(),
                    ..Default::default()
                },
            );

            let router = axum::Router::new().nest_service("/mcp", service);
            let listener = tokio::net::TcpListener::bind(&bind_addr).await?;
            axum::serve(listener, router)
                .with_graceful_shutdown(async move {
                    if let Err(e) = tokio::signal::ctrl_c().await {
                        tracing::error!("Failed to listen for Ctrl+C: {}", e);
                    }
                    ct.cancel();
                })
                .await?;
        }
        other => anyhow::bail!("Unknown transport: {}. Use 'stdio' or 'http'", other),
    }

    Ok(())
}
