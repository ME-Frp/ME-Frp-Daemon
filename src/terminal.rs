pub mod pm;
pub mod rpc;
pub mod utils;

use anyhow::Result;
use async_tungstenite::{tokio::connect_async, tungstenite::protocol::Message};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use log::{LevelFilter, error, info};
use rpc::RpcNotification;
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use std::collections::HashMap;
use std::io::{self, Write}; // Needed for stdout().flush()
use std::sync::Arc;
use tokio::sync::{Mutex, mpsc, oneshot};

use crate::rpc::{RpcMessage, RpcResponse};

/// Command line arguments parser
#[derive(Parser, Debug)]
#[clap(author, version, about, long_about = None)]
struct Cli {
  /// WebSocket server address
  #[clap(long, default_value = "127.0.0.1")]
  host: String,

  /// WebSocket server port
  #[clap(long, default_value = "9000")]
  port: u16,

  /// Or provide the full WebSocket URL directly
  #[clap(long)]
  url: Option<String>,
}

#[tokio::main]
async fn main() -> Result<(), anyhow::Error> {
  // Initialize logging
  pretty_env_logger::formatted_builder()
    .filter_level(LevelFilter::Info)
    .parse_default_env()
    .init();

  
  info!("starting terminal");

  // Parse command line arguments
  let cli = Cli::parse();

  // Determine WebSocket URL
  let ws_url = match cli.url {
    Some(url) => url,
    None => format!("ws://{}:{}", cli.host, cli.port),
  };
  info!("connecting to {}", ws_url);

  // Connect to WebSocket server
  let (ws_stream, _) = connect_async(ws_url).await?;
  info!("connected");

  // Split the WebSocket stream into read and write parts
  let (mut ws_writer, mut ws_reader) = ws_stream.split();

  // Channel to send messages from REPL to WebSocket writer task
  let (tx, mut rx) = mpsc::channel::<RpcMessage>(10);

  // Shared state to track pending requests and their response signals
  let pending_requests = Arc::new(Mutex::new(HashMap::<String, oneshot::Sender<()>>::new()));
  let pending_requests_clone = Arc::clone(&pending_requests);

  // Task to read responses from WebSocket
  let reader_task = tokio::spawn(async move {
    while let Some(msg) = ws_reader.next().await {
      match msg {
        Ok(Message::Text(text)) => {
          match serde_json::from_str::<RpcResponse>(&text) {
            Ok(response) => {
              let response_id = response.id.clone(); // Clone id for later use

              // Print response/error first
              if let Some(id) = &response.id {
                if let Some(error) = &response.error {
                  // Use print! to avoid interfering with rustyline
                  error!("[!] remote: [{}] {}", id, error);
                  let _ = io::stdout().flush(); // Flush stdout
                } else if let Some(result) = &response.result {
                  info!(
                    "[!] remote: {}",
                    serde_json::to_string_pretty(result)
                      .unwrap_or_else(|_| "<invalid JSON>".to_string())
                  );
                  let _ = io::stdout().flush(); // Flush stdout
                } else {
                  info!("[!] remote: [empty]");
                  let _ = io::stdout().flush(); // Flush stdout
                }
              } else if let Some(error) = &response.error {
                // Notifications can use error! macro
                error!("notification error: {}", error);
              }

              // If it had an ID, signal the waiting REPL loop
              if let Some(id) = response_id {
                if let Some(tx) = pending_requests_clone.lock().await.remove(&id) {
                  let _ = tx.send(()); // Send signal, ignore error
                }
              }
            }
            Err(e) => {
              // Handle notification messages
              if let Ok(message) = serde_json::from_str::<RpcNotification>(&text) {
                info!("[!] notification: {}: {:?}", message.method, message.params);
              } else {
                error!("could not parse notification: {}", e);
                error!("raw message: {}", text);
              }
            }
          }
        }
        Ok(Message::Close(_)) => {
          error!("server closed the connection");
          // Notify pending requests that connection closed?
          // For now, just break reader.
          break;
        }
        Err(e) => {
          error!("websocket error: {}", e);
          break;
        }
        _ => {} // Ignore Ping, Pong, Binary, Frame
      }
    }
    // Clear pending requests map if reader exits, dropping senders
    pending_requests_clone.lock().await.clear();
  });

  // Task to send messages from the channel to WebSocket
  let writer_task = tokio::spawn(async move {
    while let Some(rpc_msg) = rx.recv().await {
      let json = serde_json::to_string(&rpc_msg).unwrap();
      if let Err(e) = ws_writer.send(Message::Text(json.into())).await {
        error!("failed to send message: {}", e);
        // If send fails, we might want to notify the corresponding waiting REPL.
        // However, the reader task will likely error soon too.
        break;
      }
    }
  });

  // Run REPL
  info!("[!] mefrpd terminal ready");
  info!("[!] type '.help' for help, '.exit' to quit.");

  let mut rl = DefaultEditor::new()?;

  loop {
    let readline = rl.readline("> ");
    match readline {
      Ok(line) => {
        rl.add_history_entry(line.as_str())?;
        let input = line.trim();

        if input.is_empty() {
          continue;
        }

        match input {
          ".exit" | ".quit" => {
            info!("terminal exit");
            break;
          }
          ".help" => {
            info!(
              "[!] usage: <method> <params_json?>"
            );
            continue;
          }
          _ => {
            // Parse input into method name and parameters
            let parts: Vec<&str> = input.splitn(2, ' ').collect();
            let method = parts[0].to_string();
            let params = if parts.len() > 1 {
              match serde_json::from_str(parts[1]) {
                Ok(val) => Some(val),
                Err(_) => {
                  // Attempt to parse as a simple JSON string
                  match serde_json::to_value(parts[1]) {
                    Ok(val) => Some(val),
                    Err(_) => {
                      error!("invalid parameters");
                      continue;
                    }
                  }
                }
              }
            } else {
              None
            };

            let id = nanoid::nanoid!(8);
            let (response_tx, response_rx) = oneshot::channel();

            // Store the sender before sending the request
            pending_requests
              .lock()
              .await
              .insert(id.clone(), response_tx);

            // Create and send RPC message via the writer task
            let rpc_msg = RpcMessage {
              id: id.clone(),
              method,
              params,
            }; // Clone id for msg
            if let Err(e) = tx.send(rpc_msg).await {
              error!("failed to send message to writer task: {}", e);
              pending_requests.lock().await.remove(&id); // Clean up map
              break; // Exit REPL on internal send failure
            }

            // Wait for the response signal from the reader task
            match response_rx.await {
              Ok(_) => {
                // Response received and processed, continue REPL
              }
              Err(_) => {
                error!("not receive response signal");
                break; // Exit REPL if signal channel breaks
              }
            }
          }
        }
      }
      Err(ReadlineError::Interrupted) => {
        println!("^CTRL-C");
        break;
      }
      Err(ReadlineError::Eof) => {
        println!("^CTRL-D");
        break;
      }
      Err(err) => {
        error!("readline error: {:?}", err);
        break;
      }
    }
  }

  // Wait for tasks to complete (optional, cleanup)
  // Abort tasks might be cleaner if we expect them to run indefinitely
  reader_task.abort();
  writer_task.abort();
  // let _ = tokio::try_join!(reader_task, writer_task); // This would wait for them to finish naturally

  Ok(())
}
