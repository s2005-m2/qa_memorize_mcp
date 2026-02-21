use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use anyhow::Result;
use rmcp::ServiceExt;

use memorize_mcp::embedding::Embedder;
use memorize_mcp::persistence;
use memorize_mcp::server::MemorizeServer;
use memorize_mcp::storage::Storage;
use memorize_mcp::transport::ResilientStdioTransport;

struct Args {
    transport: String,
    port: u16,
    hook_port: Option<u16>,
    db_path: Option<String>,
    model_dir: String,
    debug: bool,
}

fn parse_args() -> Result<Args> {
    let args: Vec<String> = std::env::args().collect();
    let mut transport = "stdio".to_string();
    let mut port: u16 = 19532;
    let mut db_path: Option<String> = None;
    let mut model_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.join("embedding_model")))
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| "./embedding_model".to_string());
    let mut hook_port: Option<u16> = None;
    let mut debug = false;

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
            "--hook-port" => {
                i += 1;
                if i < args.len() {
                    hook_port = Some(args[i].parse().map_err(|_| {
                        anyhow::anyhow!(
                            "--hook-port value '{}' is not a valid port number",
                            args[i]
                        )
                    })?);
                }
            }
            "--db-path" => {
                i += 1;
                if i < args.len() {
                    db_path = Some(args[i].clone());
                }
            }
            "--model-dir" => {
                i += 1;
                if i < args.len() {
                    model_dir = args[i].clone();
                }
            }
            "--debug" => {
                debug = true;
            }
            "--help" | "-h" => {
                eprintln!(
                    "memorize-mcp\n\n\
                     Options:\n  \
                       --transport <stdio|http>  Transport type (default: stdio)\n  \
                       --port <PORT>             HTTP port (default: 19532)\n  \
                       --hook-port <PORT>        Start hook HTTP server for /api/recall (default: 19533 when enabled)\n  \
                       --db-path <PATH>          Database path (default: ~/.memorize-mcp)\n  \
                       --model-dir <PATH>        Embedding model directory (default: ./embedding_model)\n  \
                       --debug                   Enable debug logging to file (memorize_debug.log next to executable)"
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
        hook_port,
        db_path,
        model_dir,
        debug,
    })
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = parse_args()?;

    if args.debug {
        let log_path = std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.join("memorize_debug.log")))
            .unwrap_or_else(|| PathBuf::from("memorize_debug.log"));
        let file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&log_path)?;
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_default_env()
                    .add_directive("memorize_mcp=debug".parse().unwrap())
                    .add_directive("rmcp=debug".parse().unwrap())
                    .add_directive(tracing::Level::WARN.into()),
            )
            .with_writer(Mutex::new(file))
            .with_ansi(false)
            .init();
        tracing::info!("Debug logging to {}", log_path.display());
    } else {
        tracing_subscriber::fmt()
            .with_env_filter(
                tracing_subscriber::EnvFilter::from_default_env()
                    .add_directive(tracing::Level::INFO.into()),
            )
            .with_writer(std::io::stderr)
            .with_ansi(false)
            .init();
    }

    let data_dir: PathBuf = match &args.db_path {
        Some(p) => PathBuf::from(p),
        None => persistence::default_data_dir()?,
    };
    std::fs::create_dir_all(&data_dir)?;

    #[cfg(target_os = "windows")]
    persistence::pin_to_quick_access(&data_dir);

    let db_path_str = data_dir.to_string_lossy().to_string();

    tracing::info!("Loading embedding model from {}", args.model_dir);
    let embedder = Arc::new(Embedder::load(
        &format!("{}/model_ort.onnx", args.model_dir),
        &format!("{}/tokenizer.json", args.model_dir),
    )?);
    tracing::info!("Embedding model loaded");

    tracing::info!("Opening storage at {}", db_path_str);
    let storage = Arc::new(Storage::open(&db_path_str).await?);
    tracing::info!("Storage ready");

    tracing::info!("Syncing with JSON snapshot");
    if let Err(e) = persistence::sync_on_startup(&storage, &embedder, &data_dir).await {
        tracing::warn!("Startup sync failed (non-fatal): {}", e);
    }

    if let Err(e) = persistence::import_shared(&storage, &embedder, &data_dir).await {
        tracing::warn!("Shared import failed (non-fatal): {}", e);
    }

    let server = MemorizeServer::new(storage.clone(), embedder.clone());

    if let Some(hook_port) = args.hook_port {
        let hook_router = memorize_mcp::hook::recall_router(storage.clone(), embedder.clone());
        let mut bound_port = None;
        for offset in 0..10u16 {
            let try_port = hook_port.saturating_add(offset);
            match tokio::net::TcpListener::bind(format!("127.0.0.1:{}", try_port)).await {
                Ok(listener) => {
                    tracing::info!("Starting hook server on 127.0.0.1:{}", try_port);
                    tokio::spawn(async move {
                        axum::serve(listener, hook_router).await.unwrap();
                    });
                    bound_port = Some(try_port);
                    break;
                }
                Err(_) => {
                    tracing::warn!("Hook port {} in use, trying next", try_port);
                }
            }
        }
        if bound_port.is_none() {
            tracing::error!("Failed to bind hook server on ports {}-{}", hook_port, hook_port + 9);
        }
    }

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

            let service = StreamableHttpService::new(
                move || Ok(server.clone()),
                LocalSessionManager::default().into(),
                StreamableHttpServerConfig {
                    cancellation_token: ct.child_token(),
                    ..Default::default()
                },
            );

            let hook_router = memorize_mcp::hook::recall_router(storage.clone(), embedder.clone());
            let router = axum::Router::new()
                .nest_service("/mcp", service)
                .merge(hook_router);

            let mut listener = None;
            for offset in 0..10u16 {
                let try_port = args.port.saturating_add(offset);
                match tokio::net::TcpListener::bind(format!("127.0.0.1:{}", try_port)).await {
                    Ok(l) => {
                        tracing::info!("Starting HTTP transport on 127.0.0.1:{}", try_port);
                        listener = Some(l);
                        break;
                    }
                    Err(_) => {
                        tracing::warn!("HTTP port {} in use, trying next", try_port);
                    }
                }
            }
            let listener = listener.ok_or_else(|| {
                anyhow::anyhow!("Failed to bind HTTP on ports {}-{}", args.port, args.port + 9)
            })?;
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

    tracing::info!("Exporting JSON snapshot before shutdown");
    if let Err(e) = persistence::export_json(&storage, &data_dir).await {
        tracing::error!("Failed to export JSON on shutdown: {}", e);
    }

    Ok(())
}
