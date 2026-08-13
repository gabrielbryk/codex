/*
This module implements the remote app-server client transport.

It owns the remote connection lifecycle, including the initialize/initialized
handshake, JSON-RPC request/response routing, server-request resolution, and
notification streaming. Remote connections always carry WebSocket frames, over
either TCP WebSocket URLs or local Unix sockets. The rest of the crate uses the
same `AppServerEvent` surface for both in-process and remote transports, so
callers such as the TUI can switch between them without changing their
higher-level session logic.
*/

use std::collections::HashMap;
use std::collections::VecDeque;
use std::future::Future;
use std::io::Error as IoError;
use std::io::ErrorKind;
use std::io::Result as IoResult;
use std::time::Duration;

use crate::AppServerEvent;
use crate::RequestResult;
use crate::SHUTDOWN_TIMEOUT;
use crate::TypedRequestError;
use codex_app_server_protocol::ClientInfo;
use codex_app_server_protocol::ClientNotification;
use codex_app_server_protocol::ClientRequest;
use codex_app_server_protocol::InitializeCapabilities;
use codex_app_server_protocol::InitializeParams;
use codex_app_server_protocol::JSONRPCError;
use codex_app_server_protocol::JSONRPCErrorError;
use codex_app_server_protocol::JSONRPCMessage;
use codex_app_server_protocol::JSONRPCNotification;
use codex_app_server_protocol::JSONRPCRequest;
use codex_app_server_protocol::JSONRPCResponse;
use codex_app_server_protocol::RequestId;
use codex_app_server_protocol::Result as JsonRpcResult;
use codex_app_server_protocol::ServerNotification;
use codex_app_server_protocol::ServerRequest;
use codex_uds::UnixStream;
use codex_utils_absolute_path::AbsolutePathBuf;
use codex_utils_rustls_provider::ensure_rustls_crypto_provider;
use futures::SinkExt;
use futures::StreamExt;
use serde::de::DeserializeOwned;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use tokio::time::Instant;
use tokio::time::timeout;
use tokio_tungstenite::MaybeTlsStream;
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::client_async_with_config;
use tokio_tungstenite::connect_async_with_config;
use tokio_tungstenite::tungstenite::Error as TungsteniteError;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::tungstenite::http::HeaderValue;
use tokio_tungstenite::tungstenite::http::header::AUTHORIZATION;
use tokio_tungstenite::tungstenite::protocol::WebSocketConfig;
use tracing::warn;
use url::Url;

mod replay;

use replay::PendingRequest;
use replay::ReplayState;
use replay::earliest_parked_deadline;
use replay::expire_parked_requests;
use replay::fail_pending_requests;
use replay::park_pending_requests;

#[cfg(test)]
use replay::CLIENT_USER_MESSAGE_ID_FIELD;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const INITIALIZE_TIMEOUT: Duration = Duration::from_secs(10);
const RECONNECT_DELAY: Duration = Duration::from_millis(250);
const REMOTE_APP_SERVER_MAX_WEBSOCKET_MESSAGE_SIZE: usize = 128 << 20;
// Tungstenite still needs an HTTP request URI for the WebSocket handshake;
// the bytes travel over the Unix socket, not TCP.
const UDS_WEBSOCKET_HANDSHAKE_URL: &str = "ws://localhost/rpc";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteAppServerEndpoint {
    WebSocket {
        websocket_url: String,
        auth_token: Option<String>,
    },
    UnixSocket {
        socket_path: AbsolutePathBuf,
    },
}

#[derive(Debug, Clone)]
pub struct RemoteAppServerConnectArgs {
    pub endpoint: RemoteAppServerEndpoint,
    pub client_name: String,
    pub client_version: String,
    pub experimental_api: bool,
    pub mcp_server_openai_form_elicitation: bool,
    pub opt_out_notification_methods: Vec<String>,
    pub channel_capacity: usize,
}
impl RemoteAppServerConnectArgs {
    pub(crate) fn initialize_params(&self) -> InitializeParams {
        let capabilities = InitializeCapabilities {
            experimental_api: self.experimental_api,
            request_attestation: false,
            extensions: None,
            opt_out_notification_methods: if self.opt_out_notification_methods.is_empty() {
                None
            } else {
                Some(self.opt_out_notification_methods.clone())
            },
            mcp_server_openai_form_elicitation: self.mcp_server_openai_form_elicitation,
        };

        InitializeParams {
            client_info: ClientInfo {
                name: self.client_name.clone(),
                title: None,
                version: self.client_version.clone(),
            },
            capabilities: Some(capabilities),
        }
    }
}

pub(crate) fn websocket_url_supports_auth_token(url: &Url) -> bool {
    match (url.scheme(), url.host()) {
        ("wss", Some(_)) => true,
        ("ws", Some(url::Host::Domain(domain))) => domain.eq_ignore_ascii_case("localhost"),
        ("ws", Some(url::Host::Ipv4(addr))) => addr.is_loopback(),
        ("ws", Some(url::Host::Ipv6(addr))) => addr.is_loopback(),
        _ => false,
    }
}

enum RemoteClientCommand {
    Request {
        request: Box<JSONRPCRequest>,
        response_tx: oneshot::Sender<IoResult<RequestResult>>,
    },
    Notify {
        notification: ClientNotification,
        response_tx: oneshot::Sender<IoResult<()>>,
    },
    ResolveServerRequest {
        request_id: RequestId,
        result: JsonRpcResult,
        response_tx: oneshot::Sender<IoResult<()>>,
    },
    RejectServerRequest {
        request_id: RequestId,
        error: JSONRPCErrorError,
        response_tx: oneshot::Sender<IoResult<()>>,
    },
    Shutdown {
        response_tx: oneshot::Sender<IoResult<()>>,
    },
}

pub struct RemoteAppServerClient {
    command_tx: mpsc::Sender<RemoteClientCommand>,
    event_rx: mpsc::UnboundedReceiver<AppServerEvent>,
    pending_events: VecDeque<AppServerEvent>,
    server_version: Option<String>,
    codex_home: Option<String>,
    worker_handle: tokio::task::JoinHandle<()>,
}

#[derive(Clone)]
pub struct RemoteAppServerRequestHandle {
    command_tx: mpsc::Sender<RemoteClientCommand>,
}

impl RemoteAppServerClient {
    pub async fn connect(args: RemoteAppServerConnectArgs) -> IoResult<Self> {
        let channel_capacity = args.channel_capacity.max(1);
        let initialize_params = args.initialize_params();
        match args.endpoint {
            RemoteAppServerEndpoint::WebSocket {
                websocket_url,
                auth_token,
            } => {
                let reconnect_websocket_url = websocket_url.clone();
                let reconnect_auth_token = auth_token.clone();
                let (endpoint, stream) =
                    connect_websocket_endpoint(websocket_url, auth_token).await?;
                Self::connect_with_stream(
                    channel_capacity,
                    endpoint,
                    stream,
                    initialize_params,
                    move || {
                        connect_websocket_endpoint(
                            reconnect_websocket_url.clone(),
                            reconnect_auth_token.clone(),
                        )
                    },
                )
                .await
            }
            RemoteAppServerEndpoint::UnixSocket { socket_path } => {
                let reconnect_socket_path = socket_path.clone();
                let (endpoint, stream) = connect_unix_socket_endpoint(socket_path).await?;
                Self::connect_with_stream(
                    channel_capacity,
                    endpoint,
                    stream,
                    initialize_params,
                    move || connect_unix_socket_endpoint(reconnect_socket_path.clone()),
                )
                .await
            }
        }
    }

    pub fn server_version(&self) -> Option<&str> {
        self.server_version.as_deref()
    }

    pub fn codex_home(&self) -> Option<&str> {
        self.codex_home.as_deref()
    }

    async fn connect_with_stream<S, Reconnect, ReconnectFuture>(
        channel_capacity: usize,
        endpoint: String,
        stream: WebSocketStream<S>,
        initialize_params: InitializeParams,
        mut reconnect: Reconnect,
    ) -> IoResult<Self>
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
        Reconnect: FnMut() -> ReconnectFuture + Send + 'static,
        ReconnectFuture: Future<Output = IoResult<(String, WebSocketStream<S>)>> + Send,
    {
        let mut stream = stream;
        let (pending_events, server_version, codex_home) = initialize_remote_connection(
            &mut stream,
            &endpoint,
            initialize_params.clone(),
            INITIALIZE_TIMEOUT,
        )
        .await?;

        let (command_tx, mut command_rx) = mpsc::channel::<RemoteClientCommand>(channel_capacity);
        let (event_tx, event_rx) = mpsc::unbounded_channel::<AppServerEvent>();
        let worker_handle = tokio::spawn(async move {
            let mut pending_requests = HashMap::<RequestId, PendingRequest>::new();
            let mut endpoint = endpoint;
            let mut stream = stream;
            let mut reconnect_attempt = 0u32;
            loop {
                tokio::select! {
                    command = command_rx.recv() => {
                        let Some(command) = command else {
                            let _ = stream.close(None).await;
                            return;
                        };
                        match command {
                            RemoteClientCommand::Request { request, response_tx } => {
                                let request_id = request.id.clone();
                                if pending_requests.contains_key(&request_id) {
                                    let _ = response_tx.send(Err(IoError::new(
                                        ErrorKind::InvalidInput,
                                        format!("duplicate remote app-server request id `{request_id}`"),
                                    )));
                                    continue;
                                }
                                let replay = ReplayState::for_request(request.as_ref());
                                pending_requests.insert(
                                    request_id.clone(),
                                    PendingRequest { response_tx, replay },
                                );
                                if let Err(err) = write_jsonrpc_message(
                                    &mut stream,
                                    JSONRPCMessage::Request(*request),
                                    &endpoint,
                                )
                                .await
                                {
                                    let err_message = err.to_string();
                                    let message = format!(
                                        "remote app server at `{endpoint}` write failed: {err_message}"
                                    );
                                    // A request that never made it onto the wire is parked (and
                                    // replayed after reconnect) when it is replay-safe, and failed
                                    // together with the rest of the in-flight requests otherwise.
                                    park_pending_requests(
                                        &mut pending_requests,
                                        ErrorKind::BrokenPipe,
                                        &message,
                                    );
                                    let Some((next_endpoint, next_stream)) =
                                        reconnect_remote_stream(
                                            &mut command_rx,
                                            &event_tx,
                                            &mut reconnect,
                                            &initialize_params,
                                            &message,
                                            &mut pending_requests,
                                            &mut reconnect_attempt,
                                        )
                                        .await
                                    else {
                                        return;
                                    };
                                    endpoint = next_endpoint;
                                    stream = next_stream;
                                }
                            }
                            RemoteClientCommand::Notify { notification, response_tx } => {
                                let result = write_jsonrpc_message(
                                    &mut stream,
                                    JSONRPCMessage::Notification(
                                        jsonrpc_notification_from_client_notification(notification),
                                    ),
                                    &endpoint,
                                )
                                .await;
                                match result {
                                    Ok(()) => {
                                        let _ = response_tx.send(Ok(()));
                                    }
                                    Err(err) => {
                                        let err_message = err.to_string();
                                        let message = format!(
                                            "remote app server at `{endpoint}` write failed: {err_message}"
                                        );
                                        let _ = response_tx.send(Err(err));
                                        park_pending_requests(
                                            &mut pending_requests,
                                            ErrorKind::BrokenPipe,
                                            &message,
                                        );
                                        let Some((next_endpoint, next_stream)) =
                                            reconnect_remote_stream(
                                                &mut command_rx,
                                                &event_tx,
                                                &mut reconnect,
                                                &initialize_params,
                                                &message,
                                                &mut pending_requests,
                                                &mut reconnect_attempt,
                                            )
                                            .await
                                        else {
                                            return;
                                        };
                                        endpoint = next_endpoint;
                                        stream = next_stream;
                                    }
                                }
                            }
                            RemoteClientCommand::ResolveServerRequest {
                                request_id,
                                result,
                                response_tx,
                            } => {
                                let result = write_jsonrpc_message(
                                    &mut stream,
                                    JSONRPCMessage::Response(JSONRPCResponse {
                                        id: request_id,
                                        result,
                                    }),
                                    &endpoint,
                                )
                                .await;
                                match result {
                                    Ok(()) => {
                                        let _ = response_tx.send(Ok(()));
                                    }
                                    Err(err) => {
                                        let err_message = err.to_string();
                                        let message = format!(
                                            "remote app server at `{endpoint}` write failed: {err_message}"
                                        );
                                        let _ = response_tx.send(Err(err));
                                        park_pending_requests(
                                            &mut pending_requests,
                                            ErrorKind::BrokenPipe,
                                            &message,
                                        );
                                        let Some((next_endpoint, next_stream)) =
                                            reconnect_remote_stream(
                                                &mut command_rx,
                                                &event_tx,
                                                &mut reconnect,
                                                &initialize_params,
                                                &message,
                                                &mut pending_requests,
                                                &mut reconnect_attempt,
                                            )
                                            .await
                                        else {
                                            return;
                                        };
                                        endpoint = next_endpoint;
                                        stream = next_stream;
                                    }
                                }
                            }
                            RemoteClientCommand::RejectServerRequest {
                                request_id,
                                error,
                                response_tx,
                            } => {
                                let result = write_jsonrpc_message(
                                    &mut stream,
                                    JSONRPCMessage::Error(JSONRPCError {
                                        error,
                                        id: request_id,
                                    }),
                                    &endpoint,
                                )
                                .await;
                                match result {
                                    Ok(()) => {
                                        let _ = response_tx.send(Ok(()));
                                    }
                                    Err(err) => {
                                        let err_message = err.to_string();
                                        let message = format!(
                                            "remote app server at `{endpoint}` write failed: {err_message}"
                                        );
                                        let _ = response_tx.send(Err(err));
                                        park_pending_requests(
                                            &mut pending_requests,
                                            ErrorKind::BrokenPipe,
                                            &message,
                                        );
                                        let Some((next_endpoint, next_stream)) =
                                            reconnect_remote_stream(
                                                &mut command_rx,
                                                &event_tx,
                                                &mut reconnect,
                                                &initialize_params,
                                                &message,
                                                &mut pending_requests,
                                                &mut reconnect_attempt,
                                            )
                                            .await
                                        else {
                                            return;
                                        };
                                        endpoint = next_endpoint;
                                        stream = next_stream;
                                    }
                                }
                            }
                            RemoteClientCommand::Shutdown { response_tx } => {
                                let close_result = stream.close(None).await.or_else(|err| {
                                    if websocket_close_error_is_already_closed(&err) {
                                        Ok(())
                                    } else {
                                        Err(IoError::other(format!(
                                            "failed to close websocket app server `{endpoint}`: {err}"
                                        )))
                                    }
                                });
                                let _ = response_tx.send(close_result);
                                return;
                            }
                        }
                    }
                    message = stream.next() => {
                        match message {
                            Some(Ok(Message::Text(text))) => {
                                match serde_json::from_str::<JSONRPCMessage>(&text) {
                                    Ok(JSONRPCMessage::Response(response)) => {
                                        if let Some(pending) = pending_requests.remove(&response.id) {
                                            let _ = pending.response_tx.send(Ok(Ok(response.result)));
                                        }
                                    }
                                    Ok(JSONRPCMessage::Error(error)) => {
                                        if let Some(pending) = pending_requests.remove(&error.id) {
                                            let _ = pending.response_tx.send(Ok(Err(error.error)));
                                        }
                                    }
                                    Ok(JSONRPCMessage::Notification(notification)) => {
                                        if let Some(event) =
                                            app_server_event_from_notification(notification)
                                            && let Err(err) = deliver_event(
                                                &event_tx,
                                                event,
                                            )
                                            {
                                                warn!(%err, "failed to deliver remote app-server event");
                                                return;
                                            }
                                    }
                                    Ok(JSONRPCMessage::Request(request)) => {
                                        let request_id = request.id.clone();
                                        let method = request.method.clone();
                                        match ServerRequest::try_from(request) {
                                            Ok(request) => {
                                                if let Err(err) = deliver_event(
                                                    &event_tx,
                                                    AppServerEvent::ServerRequest(Box::new(request)),
                                                )
                                                {
                                                    warn!(%err, "failed to deliver remote app-server server request");
                                                    return;
                                                }
                                            }
                                            Err(err) => {
                                                warn!(%err, method, "rejecting unknown remote app-server request");
                                                if let Err(reject_err) = write_jsonrpc_message(
                                                    &mut stream,
                                                    JSONRPCMessage::Error(JSONRPCError {
                                                        error: JSONRPCErrorError {
                                                            code: -32601,
                                                            message: format!(
                                                                "unsupported remote app-server request `{method}`"
                                                            ),
                                                            data: None,
                                                        },
                                                        id: request_id,
                                                    }),
                                                    &endpoint,
                                                )
                                                .await
                                                {
                                                    let err_message = reject_err.to_string();
                                                    let message = format!(
                                                        "remote app server at `{endpoint}` write failed: {err_message}"
                                                    );
                                                    park_pending_requests(
                                                        &mut pending_requests,
                                                        ErrorKind::BrokenPipe,
                                                        &message,
                                                    );
                                                    let Some((next_endpoint, next_stream)) =
                                                        reconnect_remote_stream(
                                                            &mut command_rx,
                                                            &event_tx,
                                                            &mut reconnect,
                                                            &initialize_params,
                                                            &message,
                                                            &mut pending_requests,
                                                            &mut reconnect_attempt,
                                                        )
                                                        .await
                                                    else {
                                                        return;
                                                    };
                                                    endpoint = next_endpoint;
                                                    stream = next_stream;
                                                }
                                            }
                                        }
                                    }
                                    Err(err) => {
                                        let message = format!(
                                            "remote app server at `{endpoint}` sent invalid JSON-RPC: {err}"
                                        );
                                        let _ = deliver_event(
                                            &event_tx,
                                            AppServerEvent::Disconnected {
                                                message: message.clone(),
                                            },
                                        );
                                        fail_pending_requests(
                                            &mut pending_requests,
                                            ErrorKind::InvalidData,
                                            &message,
                                        );
                                        return;
                                    }
                                }
                            }
                            Some(Ok(Message::Close(frame))) => {
                                let reason = frame
                                    .as_ref()
                                    .map(|frame| frame.reason.to_string())
                                    .filter(|reason| !reason.is_empty())
                                    .unwrap_or_else(|| "connection closed".to_string());
                                let message = format!(
                                    "remote app server at `{endpoint}` disconnected: {reason}"
                                );
                                park_pending_requests(
                                    &mut pending_requests,
                                    ErrorKind::ConnectionAborted,
                                    &message,
                                );
                                let Some((next_endpoint, next_stream)) =
                                    reconnect_remote_stream(
                                        &mut command_rx,
                                        &event_tx,
                                        &mut reconnect,
                                        &initialize_params,
                                        &message,
                                        &mut pending_requests,
                                        &mut reconnect_attempt,
                                    )
                                    .await
                                else {
                                    return;
                                };
                                endpoint = next_endpoint;
                                stream = next_stream;
                            }
                            Some(Ok(Message::Binary(_)))
                            | Some(Ok(Message::Ping(_)))
                            | Some(Ok(Message::Pong(_)))
                            | Some(Ok(Message::Frame(_))) => {}
                            Some(Err(err)) => {
                                let message = format!(
                                    "remote app server at `{endpoint}` transport failed: {err}"
                                );
                                park_pending_requests(
                                    &mut pending_requests,
                                    ErrorKind::InvalidData,
                                    &message,
                                );
                                let Some((next_endpoint, next_stream)) =
                                    reconnect_remote_stream(
                                        &mut command_rx,
                                        &event_tx,
                                        &mut reconnect,
                                        &initialize_params,
                                        &message,
                                        &mut pending_requests,
                                        &mut reconnect_attempt,
                                    )
                                    .await
                                else {
                                    return;
                                };
                                endpoint = next_endpoint;
                                stream = next_stream;
                            }
                            None => {
                                let message = format!(
                                    "remote app server at `{endpoint}` closed the connection"
                                );
                                park_pending_requests(
                                    &mut pending_requests,
                                    ErrorKind::UnexpectedEof,
                                    &message,
                                );
                                let Some((next_endpoint, next_stream)) =
                                    reconnect_remote_stream(
                                        &mut command_rx,
                                        &event_tx,
                                        &mut reconnect,
                                        &initialize_params,
                                        &message,
                                        &mut pending_requests,
                                        &mut reconnect_attempt,
                                    )
                                    .await
                                else {
                                    return;
                                };
                                endpoint = next_endpoint;
                                stream = next_stream;
                            }
                        }
                    }
                }
            }
        });

        Ok(Self {
            command_tx,
            event_rx,
            pending_events: pending_events.into(),
            server_version,
            codex_home,
            worker_handle,
        })
    }

    pub fn request_handle(&self) -> RemoteAppServerRequestHandle {
        RemoteAppServerRequestHandle {
            command_tx: self.command_tx.clone(),
        }
    }

    pub async fn request(&self, request: ClientRequest) -> IoResult<RequestResult> {
        self.request_handle().request(request).await
    }

    pub async fn request_typed<T>(&self, request: ClientRequest) -> Result<T, TypedRequestError>
    where
        T: DeserializeOwned,
    {
        let method = request.method_name();
        let response =
            self.request(request)
                .await
                .map_err(|source| TypedRequestError::Transport {
                    method: method.to_string(),
                    source,
                })?;
        let result = response.map_err(|source| TypedRequestError::Server {
            method: method.to_string(),
            source,
        })?;
        serde_json::from_value(result).map_err(|source| TypedRequestError::Deserialize {
            method: method.to_string(),
            source,
        })
    }

    pub async fn notify(&self, notification: ClientNotification) -> IoResult<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(RemoteClientCommand::Notify {
                notification,
                response_tx,
            })
            .await
            .map_err(|_| {
                IoError::new(
                    ErrorKind::BrokenPipe,
                    "remote app-server worker channel is closed",
                )
            })?;
        response_rx.await.map_err(|_| {
            IoError::new(
                ErrorKind::BrokenPipe,
                "remote app-server notify channel is closed",
            )
        })?
    }

    pub async fn resolve_server_request(
        &self,
        request_id: RequestId,
        result: JsonRpcResult,
    ) -> IoResult<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(RemoteClientCommand::ResolveServerRequest {
                request_id,
                result,
                response_tx,
            })
            .await
            .map_err(|_| {
                IoError::new(
                    ErrorKind::BrokenPipe,
                    "remote app-server worker channel is closed",
                )
            })?;
        response_rx.await.map_err(|_| {
            IoError::new(
                ErrorKind::BrokenPipe,
                "remote app-server resolve channel is closed",
            )
        })?
    }

    pub async fn reject_server_request(
        &self,
        request_id: RequestId,
        error: JSONRPCErrorError,
    ) -> IoResult<()> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(RemoteClientCommand::RejectServerRequest {
                request_id,
                error,
                response_tx,
            })
            .await
            .map_err(|_| {
                IoError::new(
                    ErrorKind::BrokenPipe,
                    "remote app-server worker channel is closed",
                )
            })?;
        response_rx.await.map_err(|_| {
            IoError::new(
                ErrorKind::BrokenPipe,
                "remote app-server reject channel is closed",
            )
        })?
    }

    pub async fn next_event(&mut self) -> Option<AppServerEvent> {
        if let Some(event) = self.pending_events.pop_front() {
            return Some(event);
        }
        self.event_rx.recv().await
    }

    pub async fn shutdown(self) -> IoResult<()> {
        let Self {
            command_tx,
            event_rx,
            pending_events: _pending_events,
            server_version: _server_version,
            codex_home: _codex_home,
            worker_handle,
        } = self;
        let mut worker_handle = worker_handle;
        drop(event_rx);
        let (response_tx, response_rx) = oneshot::channel();
        if command_tx
            .send(RemoteClientCommand::Shutdown { response_tx })
            .await
            .is_ok()
            && let Ok(Ok(close_result)) = timeout(SHUTDOWN_TIMEOUT, response_rx).await
        {
            close_result?;
        }

        if let Err(_elapsed) = timeout(SHUTDOWN_TIMEOUT, &mut worker_handle).await {
            worker_handle.abort();
            let _ = worker_handle.await;
        }
        Ok(())
    }
}

async fn reconnect_remote_stream<S, Reconnect, ReconnectFuture>(
    command_rx: &mut mpsc::Receiver<RemoteClientCommand>,
    event_tx: &mpsc::UnboundedSender<AppServerEvent>,
    reconnect: &mut Reconnect,
    initialize_params: &InitializeParams,
    last_error: &str,
    pending_requests: &mut HashMap<RequestId, PendingRequest>,
    reconnect_attempt: &mut u32,
) -> Option<(String, WebSocketStream<S>)>
where
    S: AsyncRead + AsyncWrite + Unpin,
    Reconnect: FnMut() -> ReconnectFuture,
    ReconnectFuture: Future<Output = IoResult<(String, WebSocketStream<S>)>>,
{
    loop {
        *reconnect_attempt = reconnect_attempt.saturating_add(1);
        let attempt = *reconnect_attempt;
        warn!(
            attempt,
            last_error = %last_error,
            "remote app-server connection lost; reconnecting"
        );

        let reconnect_result = async {
            let (endpoint, mut stream) = reconnect().await?;
            let (pending_events, _server_version, _codex_home) = initialize_remote_connection(
                &mut stream,
                &endpoint,
                initialize_params.clone(),
                INITIALIZE_TIMEOUT,
            )
            .await?;
            Ok::<_, IoError>((endpoint, stream, pending_events))
        };

        let give_up_deadline = earliest_parked_deadline(pending_requests);
        tokio::select! {
            result = reconnect_result => {
                match result {
                    Ok((endpoint, mut stream, pending_events)) => {
                        for event in pending_events {
                            if let Err(err) = deliver_event(event_tx, event) {
                                warn!(%err, "failed to deliver queued remote app-server event after reconnect");
                                return None;
                            }
                        }
                        // Parked requests are replayed on the freshly initialized
                        // connection before the worker loop resumes, so their
                        // responses land on the original oneshot channels.
                        expire_parked_requests(pending_requests);
                        match replay_parked_requests(&mut stream, &endpoint, pending_requests).await {
                            Ok(()) => {
                                *reconnect_attempt = 0;
                                return Some((endpoint, stream));
                            }
                            Err(err) => {
                                let message = format!(
                                    "remote app server at `{endpoint}` failed to accept replayed requests: {err}"
                                );
                                warn!(attempt, error = %err, "failed to replay parked remote app-server requests");
                                // Replay attempts are already spent, so this
                                // fails the requests that cannot be retried again.
                                park_pending_requests(
                                    pending_requests,
                                    ErrorKind::BrokenPipe,
                                    &message,
                                );
                            }
                        }
                    }
                    Err(err) => {
                        warn!(
                            attempt,
                            error = %err,
                            "failed to reconnect remote app-server"
                        );
                    }
                }
            }
            command = command_rx.recv() => {
                if fail_command_while_reconnecting(command, last_error) {
                    return None;
                }
                continue;
            }
            () = sleep_until_deadline(give_up_deadline) => {
                expire_parked_requests(pending_requests);
                continue;
            }
        }

        let give_up_deadline = earliest_parked_deadline(pending_requests);
        tokio::select! {
            _ = tokio::time::sleep(RECONNECT_DELAY) => {}
            command = command_rx.recv() => {
                if fail_command_while_reconnecting(command, last_error) {
                    return None;
                }
            }
            () = sleep_until_deadline(give_up_deadline) => {
                expire_parked_requests(pending_requests);
            }
        }
    }
}

/// Sleeps until `deadline`, or forever when there is no deadline to wait on.
async fn sleep_until_deadline(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => tokio::time::sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

/// Re-sends every parked request on a freshly initialized stream.
///
/// The replayed frames reuse the original JSON-RPC ids, so responses route back
/// to the waiting oneshot channels through the normal worker-loop path. Each
/// request spends a replay attempt whether or not the write succeeds, which
/// keeps a server that dies mid-request from being retried indefinitely.
async fn replay_parked_requests<S>(
    stream: &mut WebSocketStream<S>,
    endpoint: &str,
    pending_requests: &mut HashMap<RequestId, PendingRequest>,
) -> IoResult<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let replays: Vec<(RequestId, Box<JSONRPCRequest>)> = pending_requests
        .iter_mut()
        .filter_map(|(request_id, pending)| {
            let replay = pending.replay.as_mut()?;
            replay.parked.take()?;
            replay.attempts_remaining = replay.attempts_remaining.saturating_sub(1);
            Some((request_id.clone(), replay.request.clone()))
        })
        .collect();

    for (request_id, request) in replays {
        warn!(
            %request_id,
            method = %request.method,
            "replaying remote app-server request after reconnect"
        );
        write_jsonrpc_message(stream, JSONRPCMessage::Request(*request), endpoint).await?;
    }
    Ok(())
}

fn fail_command_while_reconnecting(command: Option<RemoteClientCommand>, last_error: &str) -> bool {
    let Some(command) = command else {
        return true;
    };
    match command {
        RemoteClientCommand::Request { response_tx, .. } => {
            let _ = response_tx.send(Err(reconnecting_error(last_error)));
            false
        }
        RemoteClientCommand::Notify { response_tx, .. }
        | RemoteClientCommand::ResolveServerRequest { response_tx, .. }
        | RemoteClientCommand::RejectServerRequest { response_tx, .. } => {
            let _ = response_tx.send(Err(reconnecting_error(last_error)));
            false
        }
        RemoteClientCommand::Shutdown { response_tx } => {
            let _ = response_tx.send(Ok(()));
            true
        }
    }
}

fn reconnecting_error(last_error: &str) -> IoError {
    IoError::new(
        ErrorKind::WouldBlock,
        format!("remote app-server is reconnecting after: {last_error}"),
    )
}

impl RemoteAppServerRequestHandle {
    pub async fn request(&self, request: ClientRequest) -> IoResult<RequestResult> {
        self.request_json_rpc(jsonrpc_request_from_client_request(request))
            .await
    }

    pub async fn request_json_rpc(&self, request: JSONRPCRequest) -> IoResult<RequestResult> {
        let (response_tx, response_rx) = oneshot::channel();
        self.command_tx
            .send(RemoteClientCommand::Request {
                request: Box::new(request),
                response_tx,
            })
            .await
            .map_err(|_| {
                IoError::new(
                    ErrorKind::BrokenPipe,
                    "remote app-server worker channel is closed",
                )
            })?;
        response_rx.await.map_err(|_| {
            IoError::new(
                ErrorKind::BrokenPipe,
                "remote app-server request channel is closed",
            )
        })?
    }

    pub async fn request_typed<T>(&self, request: ClientRequest) -> Result<T, TypedRequestError>
    where
        T: DeserializeOwned,
    {
        let method = request.method_name();
        let response =
            self.request(request)
                .await
                .map_err(|source| TypedRequestError::Transport {
                    method: method.to_string(),
                    source,
                })?;
        let result = response.map_err(|source| TypedRequestError::Server {
            method: method.to_string(),
            source,
        })?;
        serde_json::from_value(result).map_err(|source| TypedRequestError::Deserialize {
            method: method.to_string(),
            source,
        })
    }
}

async fn connect_websocket_endpoint(
    websocket_url: String,
    auth_token: Option<String>,
) -> IoResult<(String, WebSocketStream<MaybeTlsStream<TcpStream>>)> {
    let url = Url::parse(&websocket_url).map_err(|err| {
        IoError::new(
            ErrorKind::InvalidInput,
            format!("invalid websocket URL `{websocket_url}`: {err}"),
        )
    })?;
    if auth_token.is_some() && !websocket_url_supports_auth_token(&url) {
        return Err(IoError::new(
            ErrorKind::InvalidInput,
            format!(
                "remote auth tokens require `wss://` or loopback `ws://` URLs; got `{websocket_url}`"
            ),
        ));
    }

    let mut request = url.as_str().into_client_request().map_err(|err| {
        IoError::new(
            ErrorKind::InvalidInput,
            format!("invalid websocket URL `{websocket_url}`: {err}"),
        )
    })?;
    if let Some(auth_token) = auth_token.as_deref() {
        let header_value =
            HeaderValue::from_str(&format!("Bearer {auth_token}")).map_err(|err| {
                IoError::new(
                    ErrorKind::InvalidInput,
                    format!("invalid remote authorization header value: {err}"),
                )
            })?;
        request.headers_mut().insert(AUTHORIZATION, header_value);
    }

    ensure_rustls_crypto_provider();
    let websocket_config = remote_websocket_config();
    let stream = timeout(
        CONNECT_TIMEOUT,
        connect_async_with_config(
            request,
            Some(websocket_config),
            /*disable_nagle*/ false,
        ),
    )
    .await
    .map_err(|_| {
        IoError::new(
            ErrorKind::TimedOut,
            format!("timed out connecting to remote app server at `{websocket_url}`"),
        )
    })?
    .map(|(stream, _response)| stream)
    .map_err(|err| {
        IoError::other(format!(
            "failed to connect to remote app server at `{websocket_url}`: {err}"
        ))
    })?;

    Ok((websocket_url, stream))
}

async fn connect_unix_socket_endpoint(
    socket_path: AbsolutePathBuf,
) -> IoResult<(String, WebSocketStream<UnixStream>)> {
    let endpoint = format!("unix://{}", socket_path.display());
    let request = UDS_WEBSOCKET_HANDSHAKE_URL
        .into_client_request()
        .map_err(|err| {
            IoError::new(
                ErrorKind::InvalidInput,
                format!("invalid UDS websocket handshake URL: {err}"),
            )
        })?;
    let stream = timeout(CONNECT_TIMEOUT, UnixStream::connect(socket_path.as_path()))
        .await
        .map_err(|_| {
            IoError::new(
                ErrorKind::TimedOut,
                format!("timed out connecting to remote app server at `{endpoint}`"),
            )
        })?
        .map_err(|err| {
            IoError::other(format!(
                "failed to connect to remote app server at `{endpoint}`: {err}"
            ))
        })?;
    let websocket_config = remote_websocket_config();
    let stream = timeout(
        CONNECT_TIMEOUT,
        client_async_with_config(request, stream, Some(websocket_config)),
    )
    .await
    .map_err(|_| {
        IoError::new(
            ErrorKind::TimedOut,
            format!("timed out upgrading remote app server at `{endpoint}`"),
        )
    })?
    .map(|(stream, _response)| stream)
    .map_err(|err| {
        IoError::other(format!(
            "failed to upgrade remote app server at `{endpoint}`: {err}"
        ))
    })?;

    Ok((endpoint, stream))
}

fn remote_websocket_config() -> WebSocketConfig {
    WebSocketConfig::default()
        .max_frame_size(Some(REMOTE_APP_SERVER_MAX_WEBSOCKET_MESSAGE_SIZE))
        .max_message_size(Some(REMOTE_APP_SERVER_MAX_WEBSOCKET_MESSAGE_SIZE))
}

async fn initialize_remote_connection<S>(
    stream: &mut WebSocketStream<S>,
    endpoint: &str,
    params: InitializeParams,
    initialize_timeout: Duration,
) -> IoResult<(Vec<AppServerEvent>, Option<String>, Option<String>)>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let initialize_request_id = RequestId::String("initialize".to_string());
    let mut pending_events = Vec::new();
    let mut server_version = None;
    let mut codex_home = None;
    write_jsonrpc_message(
        stream,
        JSONRPCMessage::Request(jsonrpc_request_from_client_request(
            ClientRequest::Initialize {
                request_id: initialize_request_id.clone(),
                params,
            },
        )),
        endpoint,
    )
    .await?;

    timeout(initialize_timeout, async {
        loop {
            match stream.next().await {
                Some(Ok(Message::Text(text))) => {
                    let message = serde_json::from_str::<JSONRPCMessage>(&text).map_err(|err| {
                        IoError::other(format!(
                            "remote app server at `{endpoint}` sent invalid initialize response: {err}"
                        ))
                    })?;
                    match message {
                        JSONRPCMessage::Response(response) if response.id == initialize_request_id => {
                            server_version = response
                                .result
                                .get("userAgent")
                                .and_then(serde_json::Value::as_str)
                                .and_then(|user_agent| {
                                    let (_, rest) = user_agent.split_once('/')?;
                                    rest.split_whitespace().next().map(str::to_string)
                                });
                            codex_home = response
                                .result
                                .get("codexHome")
                                .and_then(serde_json::Value::as_str)
                                .filter(|codex_home| !codex_home.is_empty())
                                .map(str::to_string);
                            break Ok(());
                        }
                        JSONRPCMessage::Error(error) if error.id == initialize_request_id => {
                            break Err(IoError::other(format!(
                                "remote app server at `{endpoint}` rejected initialize: {}",
                                error.error.message
                            )));
                        }
                        JSONRPCMessage::Notification(notification) => {
                            if let Some(event) = app_server_event_from_notification(notification) {
                                pending_events.push(event);
                            }
                        }
                        JSONRPCMessage::Request(request) => {
                            let request_id = request.id.clone();
                            let method = request.method.clone();
                            match ServerRequest::try_from(request) {
                                Ok(request) => {
                                    pending_events
                                        .push(AppServerEvent::ServerRequest(Box::new(request)));
                                }
                                Err(err) => {
                                    warn!(%err, method, "rejecting unknown remote app-server request during initialize");
                                    write_jsonrpc_message(
                                        stream,
                                        JSONRPCMessage::Error(JSONRPCError {
                                            error: JSONRPCErrorError {
                                                code: -32601,
                                                message: format!(
                                                    "unsupported remote app-server request `{method}`"
                                                ),
                                                data: None,
                                            },
                                            id: request_id,
                                        }),
                                        endpoint,
                                    )
                                    .await?;
                                }
                            }
                        }
                        JSONRPCMessage::Response(_) | JSONRPCMessage::Error(_) => {}
                    }
                }
                Some(Ok(Message::Binary(_)))
                | Some(Ok(Message::Ping(_)))
                | Some(Ok(Message::Pong(_)))
                | Some(Ok(Message::Frame(_))) => {}
                Some(Ok(Message::Close(frame))) => {
                    let reason = frame
                        .as_ref()
                        .map(|frame| frame.reason.to_string())
                        .filter(|reason| !reason.is_empty())
                        .unwrap_or_else(|| "connection closed during initialize".to_string());
                    break Err(IoError::new(
                        ErrorKind::ConnectionAborted,
                        format!(
                            "remote app server at `{endpoint}` closed during initialize: {reason}"
                        ),
                    ));
                }
                Some(Err(err)) => {
                    break Err(IoError::other(format!(
                        "remote app server at `{endpoint}` transport failed during initialize: {err}"
                    )));
                }
                None => {
                    break Err(IoError::new(
                        ErrorKind::UnexpectedEof,
                        format!("remote app server at `{endpoint}` closed during initialize"),
                    ));
                }
            }
        }
    })
    .await
    .map_err(|_| {
        IoError::new(
            ErrorKind::TimedOut,
            format!("timed out waiting for initialize response from `{endpoint}`"),
        )
    })??;

    write_jsonrpc_message(
        stream,
        JSONRPCMessage::Notification(jsonrpc_notification_from_client_notification(
            ClientNotification::Initialized,
        )),
        endpoint,
    )
    .await?;

    Ok((pending_events, server_version, codex_home))
}

fn app_server_event_from_notification(notification: JSONRPCNotification) -> Option<AppServerEvent> {
    match ServerNotification::try_from(notification) {
        Ok(notification) => Some(AppServerEvent::ServerNotification(Box::new(notification))),
        Err(_) => None,
    }
}

fn deliver_event(
    event_tx: &mpsc::UnboundedSender<AppServerEvent>,
    event: AppServerEvent,
) -> IoResult<()> {
    event_tx.send(event).map_err(|_| {
        IoError::new(
            ErrorKind::BrokenPipe,
            "remote app-server event consumer channel is closed",
        )
    })
}

fn jsonrpc_request_from_client_request(request: ClientRequest) -> JSONRPCRequest {
    let value = match serde_json::to_value(request) {
        Ok(value) => value,
        Err(err) => panic!("client request should serialize: {err}"),
    };
    match serde_json::from_value(value) {
        Ok(request) => request,
        Err(err) => panic!("client request should encode as JSON-RPC request: {err}"),
    }
}

fn jsonrpc_notification_from_client_notification(
    notification: ClientNotification,
) -> JSONRPCNotification {
    let value = match serde_json::to_value(notification) {
        Ok(value) => value,
        Err(err) => panic!("client notification should serialize: {err}"),
    };
    match serde_json::from_value(value) {
        Ok(notification) => notification,
        Err(err) => panic!("client notification should encode as JSON-RPC notification: {err}"),
    }
}

async fn write_jsonrpc_message<S>(
    stream: &mut WebSocketStream<S>,
    message: JSONRPCMessage,
    endpoint: &str,
) -> IoResult<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let payload = serde_json::to_string(&message).map_err(IoError::other)?;
    stream
        .send(Message::Text(payload.into()))
        .await
        .map_err(|err| {
            IoError::other(format!(
                "failed to write websocket message to `{endpoint}`: {err}"
            ))
        })
}

fn websocket_close_error_is_already_closed(err: &TungsteniteError) -> bool {
    match err {
        TungsteniteError::ConnectionClosed | TungsteniteError::AlreadyClosed => true,
        TungsteniteError::Io(err) => matches!(
            err.kind(),
            ErrorKind::BrokenPipe | ErrorKind::ConnectionReset | ErrorKind::NotConnected
        ),
        _ => false,
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn shutdown_tolerates_worker_exit_after_command_is_queued() {
        let (command_tx, mut command_rx) = mpsc::channel(1);
        let (_event_tx, event_rx) = mpsc::unbounded_channel::<AppServerEvent>();
        let worker_handle = tokio::spawn(async move {
            let _ = command_rx.recv().await;
        });
        let client = RemoteAppServerClient {
            command_tx,
            event_rx,
            pending_events: VecDeque::new(),
            server_version: None,
            codex_home: None,
            worker_handle,
        };

        client
            .shutdown()
            .await
            .expect("shutdown should complete when worker exits first");
    }

    fn jsonrpc_request(id: i64, method: &str, params: serde_json::Value) -> JSONRPCRequest {
        JSONRPCRequest {
            id: RequestId::Integer(id),
            method: method.to_string(),
            params: Some(params),
            trace: None,
        }
    }

    fn turn_start_jsonrpc_request(id: i64, client_user_message_id: Option<&str>) -> JSONRPCRequest {
        let mut params = serde_json::json!({ "threadId": "thread" });
        if let Some(client_user_message_id) = client_user_message_id {
            params[CLIENT_USER_MESSAGE_ID_FIELD] = serde_json::json!(client_user_message_id);
        }
        jsonrpc_request(id, "turn/start", params)
    }

    /// Registers `request` as in-flight and returns the receiver its caller
    /// would be awaiting.
    fn track_request(
        pending_requests: &mut HashMap<RequestId, PendingRequest>,
        request: JSONRPCRequest,
    ) -> oneshot::Receiver<IoResult<RequestResult>> {
        let (response_tx, response_rx) = oneshot::channel();
        let replay = ReplayState::for_request(&request);
        pending_requests.insert(
            request.id,
            PendingRequest {
                response_tx,
                replay,
            },
        );
        response_rx
    }

    #[test]
    fn replay_is_limited_to_idempotent_turn_start_requests() {
        assert!(
            ReplayState::for_request(&turn_start_jsonrpc_request(1, Some("submission-1")))
                .is_some(),
            "turn/start carrying an idempotency key should be replayable"
        );
        assert!(
            ReplayState::for_request(&turn_start_jsonrpc_request(2, None)).is_none(),
            "turn/start without an idempotency key must not be replayed"
        );
        assert!(
            ReplayState::for_request(&turn_start_jsonrpc_request(3, Some(""))).is_none(),
            "an empty idempotency key must not be treated as a key"
        );
        assert!(
            ReplayState::for_request(&jsonrpc_request(
                4,
                "turn/interrupt",
                serde_json::json!({ CLIENT_USER_MESSAGE_ID_FIELD: "submission-1" }),
            ))
            .is_none(),
            "only allowlisted methods may be replayed"
        );
    }

    #[tokio::test]
    async fn park_keeps_replayable_requests_and_fails_the_rest() {
        let mut pending_requests = HashMap::new();
        let mut replayable = track_request(
            &mut pending_requests,
            turn_start_jsonrpc_request(1, Some("submission-1")),
        );
        let mut not_replayable = track_request(
            &mut pending_requests,
            jsonrpc_request(2, "account/read", serde_json::json!({})),
        );

        park_pending_requests(
            &mut pending_requests,
            ErrorKind::ConnectionAborted,
            "server disconnected",
        );

        assert_eq!(
            pending_requests.len(),
            1,
            "replayable request should be parked"
        );
        assert!(
            replayable.try_recv().is_err(),
            "parked request must not be answered yet"
        );
        let err = not_replayable
            .try_recv()
            .expect("non-replayable request should be answered immediately")
            .expect_err("non-replayable request should fail");
        assert_eq!(err.kind(), ErrorKind::ConnectionAborted);
        assert_eq!(err.to_string(), "server disconnected");
    }

    #[tokio::test(start_paused = true)]
    async fn parked_requests_give_up_with_the_original_transport_error() {
        let mut pending_requests = HashMap::new();
        let mut parked = track_request(
            &mut pending_requests,
            turn_start_jsonrpc_request(1, Some("submission-1")),
        );

        park_pending_requests(
            &mut pending_requests,
            ErrorKind::UnexpectedEof,
            "server closed the connection",
        );
        let deadline = earliest_parked_deadline(&pending_requests)
            .expect("a parked request should have a give-up deadline");

        tokio::time::sleep_until(deadline).await;
        expire_parked_requests(&mut pending_requests);

        assert!(
            pending_requests.is_empty(),
            "expired request should be dropped"
        );
        let err = parked
            .try_recv()
            .expect("expired request should be answered")
            .expect_err("expired request should fail");
        assert_eq!(err.kind(), ErrorKind::UnexpectedEof);
        assert_eq!(
            err.to_string(),
            "server closed the connection",
            "give-up should report the transport error that parked the request"
        );
    }

    #[tokio::test]
    async fn a_replayed_request_is_not_parked_a_second_time() {
        let mut pending_requests = HashMap::new();
        let mut request = track_request(
            &mut pending_requests,
            turn_start_jsonrpc_request(1, Some("submission-1")),
        );

        park_pending_requests(&mut pending_requests, ErrorKind::BrokenPipe, "first drop");
        let (client_side, _server_side) = tokio::io::duplex(64 * 1024);
        let mut stream = WebSocketStream::from_raw_socket(
            client_side,
            tokio_tungstenite::tungstenite::protocol::Role::Client,
            None,
        )
        .await;
        replay_parked_requests(&mut stream, "test-endpoint", &mut pending_requests)
            .await
            .expect("replay should write to a healthy stream");
        assert_eq!(
            pending_requests.len(),
            1,
            "replayed request stays in flight"
        );
        assert!(
            earliest_parked_deadline(&pending_requests).is_none(),
            "a replayed request is no longer parked"
        );

        park_pending_requests(&mut pending_requests, ErrorKind::BrokenPipe, "second drop");

        assert!(
            pending_requests.is_empty(),
            "a request may only be replayed once"
        );
        let err = request
            .try_recv()
            .expect("exhausted request should be answered")
            .expect_err("exhausted request should fail");
        assert_eq!(err.kind(), ErrorKind::BrokenPipe);
        assert_eq!(err.to_string(), "second drop");
    }
}
