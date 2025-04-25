pub mod rpc;
pub mod utils;

use std::{net::{Ipv4Addr, SocketAddr, SocketAddrV4}, sync::Arc};
use reserve_port::find_unused_port;
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use log::{error, info, LevelFilter, warn, debug};
use serde_json::Value;

use rpc::{store, RpcMethodRegistry, RpcServer};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
  // Initialize logging system
  pretty_env_logger::formatted_builder()
    .filter_level(LevelFilter::Info)
    .parse_env("FRPD_LOG")
    .init();
  
  info!("Starting ME-Frp-Daemon");
  debug!("Initializing store");
  store::set(1)?;

  info!("Setting up RPC registry");
  let mut registry = RpcMethodRegistry::new();
  
  debug!("Registering methods");
  rpc_handler!(registry, {
    "hello" => |_params, _server| async {
      let a = store::get::<i32>()?.unwrap();
      info!("Hello, world! {}", a);
      Ok(Value::String("Hello, world!".to_string()))
    },
  });

  debug!("Creating RPC server");
  let server = RpcServer::new(Arc::new(Mutex::new(registry)));

  info!("Finding available port");
  let port = find_unused_port().unwrap();
  info!("Port found: {}", port);

  info!("Binding to port {}", port);
  let listener = match TcpListener::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))).await {
    Ok(l) => l,
    Err(e) => {
      error!("Bind failed on port {}: {}", port, e);
      return Err(anyhow::anyhow!("Failed to bind to port {}: {}", port, e));
    }
  };
  info!("Server listening on: {}", listener.local_addr()?);

  info!("Starting server loop");
  match server.run(listener).await {
    Ok(_) => {
      info!("Server terminated successfully");
      Ok(())
    },
    Err(e) => {
      error!("Server error: {}", e);
      Err(e)
    }
  }
}
