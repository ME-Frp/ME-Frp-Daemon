pub mod rpc;
pub mod utils;

use std::{net::{Ipv4Addr, SocketAddr, SocketAddrV4}, sync::Arc};
use reserve_port::{find_unused_port, is_port_available};
use tokio::net::TcpListener;
use tokio::sync::Mutex;
use log::{error, info, LevelFilter, debug};

use rpc::{store, RpcMethodRegistry, RpcServer};

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
  // Initialize logging system
  pretty_env_logger::formatted_builder()
    .filter_level(LevelFilter::Info)
    .parse_env("FRPD_LOG")
    .init();
  
  info!("starting me-frp-daemon");
  debug!("initializing store");
  store::set(1)?;

  info!("setting up rpc registry");
  let mut registry = RpcMethodRegistry::new();
  
  debug!("registering methods");
  rpc_handler!(registry, {
    "hello" => |_params, _server| async {
      let a = store::get::<i32>()?.unwrap();
      info!("hello, world! {}", a);
      Ok(Value::String("Hello, world!".to_string()))
    },

    "ping" => |_params, _server| async {
      Ok(Value::Null)
    }
  });

  debug!("creating frpd instance");
  let server = RpcServer::new(Arc::new(Mutex::new(registry)));

  info!("finding available port");

  let port = {
    if is_port_available(62000) {
      info!("default port is available");
      62000
    } else {
      find_unused_port().unwrap()
    }
  };

  debug!("port: {}", port);

  debug!("binding to port {}", port);
  let listener = match TcpListener::bind(SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port))).await {
    Ok(l) => l,
    Err(e) => {
      error!("bind failed on port {}: {}", port, e);
      return Err(anyhow::anyhow!("Failed to bind to port {}: {}", port, e));
    }
  };
  info!("frpd listening on: {}", listener.local_addr()?);

  match server.run(listener).await {
    Ok(_) => {
      info!("frpd terminated successfully");
      Ok(())
    },
    Err(e) => {
      error!("frpd error: {}", e);
      Err(e)
    }
  }
}
