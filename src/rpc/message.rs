use serde::{Deserialize, Serialize};
use serde_json::Value;
use log::{debug, trace};

/// Represents an incoming JSON-RPC request
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RpcMessage {
  /// Unique identifier for the request
  pub id: String,
  /// Name of the RPC method to call
  pub method: String,
  /// Optional parameters for the method
  #[serde(skip_serializing_if = "Option::is_none")]
  pub params: Option<Value>,
}

/// Represents a JSON-RPC response
#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct RpcResponse {
  /// Identifier matching the request, or None for notifications
  #[serde(skip_serializing_if = "Option::is_none")]
  pub id: Option<String>,
  /// Result of the method call if successful
  #[serde(skip_serializing_if = "Option::is_none")]
  pub result: Option<Value>,
  /// Error information if the call failed
  #[serde(skip_serializing_if = "Option::is_none")]
  pub error: Option<String>,
}

impl RpcResponse {
  /// Create a new RPC response with the given id, result, and error
  pub fn new(id: String, result: Option<Value>, error: Option<String>) -> Self {
    debug!("response created: id={}", id);
    Self { id: Some(id), result, error }
  }

  /// Create an error response with the given id
  pub fn from_error(id: String, error: anyhow::Error) -> Self {
    debug!("error response: id={}, error={}", id, error);
    Self { id: Some(id), result: None, error: Some(error.to_string().into()) }
  }

  /// Create a success response with the given id and value
  pub fn from_value(id: String, value: Option<Value>) -> Self {
    debug!("success response: id={}", id);
    trace!("value: {:?}", value);
    Self { id: Some(id), result: value, error: None }
  }

  /// Create an error notification (no id)
  pub fn notification_error(error: anyhow::Error) -> Self {
    debug!("error notification: {}", error);
    Self { id: None, result: None, error: Some(error.to_string().into()) }
  }

  /// Create a response from a Result, handling both success and error cases
  pub fn from_result(id: String, result: Result<Value, anyhow::Error>) -> Self {
    match result {
      Ok(value) => Self::from_value(id, if value.is_null() { None } else { Some(value) }),
      Err(error) => Self::from_error(id, error),
    }
  }
}
