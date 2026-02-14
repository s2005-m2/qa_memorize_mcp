use std::sync::Arc;

use futures::{SinkExt, StreamExt};
use rmcp::{
    RoleServer,
    model::*,
    service::{RxJsonRpcMessage, TxJsonRpcMessage},
    transport::Transport,
    transport::async_rw::JsonRpcMessageCodec,
};
use tokio::sync::Mutex;
use tokio_util::{
    bytes::BytesMut,
    codec::{Decoder, FramedRead, FramedWrite},
};

type ServerRx = RxJsonRpcMessage<RoleServer>;
type ServerTx = TxJsonRpcMessage<RoleServer>;
type Writer = FramedWrite<tokio::io::Stdout, JsonRpcMessageCodec<ServerTx>>;

// ── Resilient Decoder ──

enum DecodeResult {
    Message(ServerRx),
    ParseError { raw: String, error: String },
}

struct ResilientCodec {
    inner: JsonRpcMessageCodec<ServerRx>,
}

impl ResilientCodec {
    fn new() -> Self {
        Self {
            inner: JsonRpcMessageCodec::default(),
        }
    }
}

impl Decoder for ResilientCodec {
    type Item = DecodeResult;
    type Error = std::io::Error;

    fn decode(&mut self, buf: &mut BytesMut) -> Result<Option<Self::Item>, Self::Error> {
        let snapshot = buf.clone();
        match self.inner.decode(buf) {
            Ok(Some(msg)) => Ok(Some(DecodeResult::Message(msg))),
            Ok(None) => Ok(None),
            Err(e) => {
                let consumed = snapshot.len() - buf.len();
                let raw = String::from_utf8_lossy(&snapshot[..consumed]).trim().to_string();
                Ok(Some(DecodeResult::ParseError {
                    raw,
                    error: e.to_string(),
                }))
            }
        }
    }
}

// ── Transport ──

pub struct ResilientStdioTransport {
    read: Arc<Mutex<FramedRead<tokio::io::Stdin, ResilientCodec>>>,
    write: Arc<Mutex<Option<Writer>>>,
}

impl ResilientStdioTransport {
    pub fn new() -> Self {
        Self {
            read: Arc::new(Mutex::new(FramedRead::new(
                tokio::io::stdin(),
                ResilientCodec::new(),
            ))),
            write: Arc::new(Mutex::new(Some(FramedWrite::new(
                tokio::io::stdout(),
                JsonRpcMessageCodec::<ServerTx>::default(),
            )))),
        }
    }

    async fn send_parse_error(write: &Arc<Mutex<Option<Writer>>>, raw: &str, error: &str) {
        let id = serde_json::from_str::<serde_json::Value>(raw)
            .ok()
            .and_then(|v| v.get("id").cloned())
            .and_then(|id| serde_json::from_value::<RequestId>(id).ok());

        let truncated_raw = if raw.len() > 200 {
            format!("{}...", &raw[..200])
        } else {
            raw.to_string()
        };

        let error_msg: ServerTx = JsonRpcMessage::Error(JsonRpcError {
            jsonrpc: JsonRpcVersion2_0,
            id: id.unwrap_or(RequestId::Number(0)),
            error: ErrorData::new(
                ErrorCode::PARSE_ERROR,
                format!(
                    "Failed to parse JSON-RPC message: {}. \
                     Ensure your request is valid JSON-RPC 2.0 conforming to the MCP protocol. \
                     Raw input: {}",
                    error, truncated_raw
                ),
                None,
            ),
        });

        let mut guard = write.lock().await;
        if let Some(ref mut w) = *guard {
            if let Err(e) = w.send(error_msg).await {
                tracing::error!("Failed to send parse error response: {}", e);
            }
        }
    }
}

impl Transport<RoleServer> for ResilientStdioTransport {
    type Error = std::io::Error;

    fn send(
        &mut self,
        item: ServerTx,
    ) -> impl Future<Output = Result<(), Self::Error>> + Send + 'static {
        let lock = self.write.clone();
        async move {
            let mut guard = lock.lock().await;
            if let Some(ref mut w) = *guard {
                w.send(item).await.map_err(Into::into)
            } else {
                Err(std::io::Error::new(
                    std::io::ErrorKind::NotConnected,
                    "Transport is closed",
                ))
            }
        }
    }

    fn receive(&mut self) -> impl Future<Output = Option<ServerRx>> + Send {
        let read = self.read.clone();
        let write = self.write.clone();
        async move {
            let mut reader = read.lock().await;
            loop {
                match reader.next().await {
                    Some(Ok(DecodeResult::Message(msg))) => return Some(msg),
                    Some(Ok(DecodeResult::ParseError { raw, error })) => {
                        tracing::warn!(
                            "Malformed JSON-RPC message ({}), sending error response to client",
                            error
                        );
                        Self::send_parse_error(&write, &raw, &error).await;
                    }
                    Some(Err(e)) => {
                        tracing::error!("Stdio read error: {}", e);
                        return None;
                    }
                    None => return None,
                }
            }
        }
    }

    async fn close(&mut self) -> Result<(), Self::Error> {
        let mut guard = self.write.lock().await;
        drop(guard.take());
        Ok(())
    }
}
