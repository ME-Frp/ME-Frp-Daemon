use super::{RpcMessage, RpcMethodRegistry, RpcResponse};
use anyhow::anyhow;
use async_tungstenite::tokio::accept_async;
use async_tungstenite::tungstenite::Message;
use futures_channel::{mpsc::UnboundedSender, mpsc::unbounded};
use futures_util::{SinkExt, StreamExt};
use log::{debug, error, info, warn, trace};
use serde_json::Value;
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;


type Tx = UnboundedSender<Message>;
type PeerMap = Arc<Mutex<HashMap<SocketAddr, Tx>>>;

/// RPC Server implementation providing WebSocket communication
pub struct RpcServer {
  /// Registry containing available RPC methods
  registry: Arc<Mutex<RpcMethodRegistry>>,
  /// Map of connected clients
  peers: PeerMap,
}

impl RpcServer {
  /// Create a new RPC server with the given method registry
  pub fn new(registry: Arc<Mutex<RpcMethodRegistry>>) -> Self {
    info!("frpd init");
    Self {
      registry,
      peers: Arc::new(Mutex::new(HashMap::new())),
    }
  }

  /// Broadcast a message to all connected clients, with optional exclusion
  pub async fn broadcast(
    &self,
    message: Message,
    exclude: Option<&SocketAddr>,
  ) -> Result<(), anyhow::Error> {
    let peer_count = {
      let peers = self.peers.lock().await;
      debug!("broadcasting to {} clients", peers.len());
      trace!("message: {:?}", message);
      
      if let Some(ex) = exclude {
        debug!("excluding {}", ex);
      }

      let broadcast_recipients = peers
        .iter()
        .filter(|(peer_addr, _)| match exclude {
          Some(ex) => peer_addr != &ex,
          None => true,
        })
        .map(|(_, ws_sink)| ws_sink);

      let mut send_count = 0;
      for recipient in broadcast_recipients {
        if let Err(e) = recipient.unbounded_send(message.clone()) {
          warn!("broadcast failed: {}", e);
        } else {
          send_count += 1;
        }
      }
      send_count
    };
    
    debug!("broadcast complete: {} clients", peer_count);
    Ok(())
  }

  /// Handle a new client connection
  async fn handle_connection(
    registry: Arc<Mutex<RpcMethodRegistry>>,
    peers: PeerMap,
    stream: TcpStream,
    addr: SocketAddr,
    server: Arc<RpcServer>,
  ) {
    info!("tcp connection from: {}", addr);
    let ws_stream = match accept_async(stream).await {
      Ok(ws) => ws,
      Err(e) => {
        error!("ws handshake failed: {}: {}", addr, e);
        return;
      }
    };
    info!("ws connection established: {}", addr);

    let (mut writer, mut reader) = ws_stream.split();
    let (tx, mut rx) = unbounded::<Message>();

    {
      let mut peers_lock = peers.lock().await;
      peers_lock.insert(addr, tx.clone());
      info!("client {} connected (total: {})", addr, peers_lock.len());
    }

    let registry_clone = Arc::clone(&registry);
    let tx_clone = tx.clone();
    let server_clone = Arc::clone(&server);

    debug!("starting reader for {}", addr);
    let read_task = tokio::spawn(async move {
      while let Some(message_result) = reader.next().await {
        let message = match message_result {
          Ok(msg) => msg,
          Err(e) => {
            error!("read error {}: {}", addr, e);
            break;
          }
        };

        let registry_inner = Arc::clone(&registry_clone);
        let tx_inner = tx_clone.clone();
        let server_inner = Arc::clone(&server_clone);
        
        trace!("processing message from {}", addr);
        tokio::spawn(async move {
          let result: Result<(), anyhow::Error> =
            process_message(registry_inner, tx_inner, message, addr, server_inner).await;
          if let Err(e) = result {
            error!("process error {}: {}", addr, e);
          }
        });
      }
      debug!("reader stopped for {}", addr);
    });

    debug!("starting writer for {}", addr);
    let write_task = tokio::spawn(async move {
      while let Some(message) = rx.next().await {
        trace!("sending to {}: {:?}", addr, message);
        if writer.send(message).await.is_err() {
          debug!("send failed to {}", addr);
          break;
        }
      }
      debug!("writer stopped for {}", addr);
    });

    let _ = read_task.await;
    write_task.abort();

    {
      let mut peers_lock = peers.lock().await;
      peers_lock.remove(&addr);
      info!("client {} disconnected (total: {})", addr, peers_lock.len());
    }
  }

  /// Start the RPC server and listen for incoming connections
  pub async fn run(&self, listener: TcpListener) -> Result<(), anyhow::Error> {
    let addr = listener.local_addr()?;
    info!("listening on {}", addr);

    let server = Arc::new(self.clone());
    debug!("connection loop started");
    loop {
      match listener.accept().await {
        Ok((stream, addr)) => {
          debug!("connection from {}", addr);
          tokio::spawn(Self::handle_connection(
            Arc::clone(&self.registry),
            Arc::clone(&self.peers),
            stream,
            addr,
            Arc::clone(&server),
          ));
        }
        Err(e) => {
          error!("accept failed: {}", e);
        }
      }
    }
  }

  fn clone(&self) -> Self {
    trace!("cloning frpd");
    Self {
      registry: Arc::clone(&self.registry),
      peers: Arc::clone(&self.peers),
    }
  }
}

/// Process a single RPC message
async fn process_message(
  registry: Arc<Mutex<RpcMethodRegistry>>,
  tx: Tx,
  message: Message,
  addr: SocketAddr,
  server: Arc<RpcServer>,
) -> Result<(), anyhow::Error> {
  match message {
    Message::Text(text) => {
      debug!("text from {}: {}", addr, text);
      let rpc_message: RpcMessage = match serde_json::from_str::<RpcMessage>(&text) {
        Ok(m) => {
          debug!("parsed: method={}, id={}", m.method, m.id);
          m
        },
        Err(e) => {
          warn!("parse error {}: {}", addr, e);
          let err_response =
            RpcResponse::notification_error(anyhow!("invalid json received"));
          let response_str = serde_json::to_string(&err_response)?;
          debug!("error response: {}", response_str);
          tx.unbounded_send(Message::Text(response_str.into()))?;
          return Ok(());
        }
      };
      let id = rpc_message.id.clone();

      debug!("calling '{}' (id: '{}')", rpc_message.method, id);
      let result: Result<Value, anyhow::Error> = {
        let registry_lock = registry.lock().await;
        registry_lock.call(&rpc_message, server).await
      };

      match &result {
        Ok(_) => debug!("'{}' succeeded", rpc_message.method),
        Err(e) => warn!("'{}' failed: {}", rpc_message.method, e),
      }

      let response = RpcResponse::from_result(id, result);
      let response_str = serde_json::to_string(&response)?;
      debug!("response to {}: {}", addr, response_str);
      tx.unbounded_send(Message::Text(response_str.into()))?;
    }
    Message::Binary(data) => {
      debug!("binary from {}: {} bytes", addr, data.len());
    }
    Message::Ping(data) => {
      debug!("ping from {}", addr);
      tx.unbounded_send(Message::Pong(data))?;
    }
    Message::Pong(_) => {
      trace!("pong from {}", addr);
    }
    Message::Close(reason) => {
      info!("close from {}: {:?}", addr, reason);
    }
    Message::Frame(_) => {
      trace!("frame from {}", addr);
    }
  }
  Ok(())
}
