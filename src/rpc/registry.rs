use anyhow::anyhow;
use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::RwLock;
use log::{debug, warn, error};

use serde_json::Value;

use super::{RpcMessage, RpcServer};
use once_cell::sync::Lazy;

/// Global type-based storage for sharing data across RPC handlers
static GLOBAL_STORE: Lazy<Arc<RwLock<HashMap<TypeId, Arc<dyn Any + Send + Sync>>>>> =
  Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

/// Storage utilities for accessing and modifying the global store
pub mod store {
  use super::*;

  /// Get a value from the global store by its type
  pub fn get<T: 'static + Send + Sync>() -> Result<Option<Arc<T>>, anyhow::Error> {
    debug!("getting {:?} from store", std::any::type_name::<T>());
    let store = GLOBAL_STORE
      .read()
      .map_err(|e| {
        error!("read lock failed: {:?}", e);
        anyhow!("Failed to acquire read lock")
      })?;

    let result = store
      .get(&TypeId::of::<T>())
      .and_then(|arc| Arc::downcast::<T>(Arc::clone(arc)).ok());
    
    if result.is_some() {
      debug!("found {:?} in store", std::any::type_name::<T>());
    } else {
      debug!("{:?} not in store", std::any::type_name::<T>());
    }
    
    Ok(result)
  }

  /// Set a value in the global store, indexed by its type
  pub fn set<T: 'static + Send + Sync>(value: T) -> Result<(), anyhow::Error> {
    debug!("setting {:?} in store", std::any::type_name::<T>());
    let mut store = GLOBAL_STORE
      .write()
      .map_err(|e| {
        error!("write lock failed: {:?}", e);
        anyhow!("Failed to acquire write lock")
      })?;

    store.insert(TypeId::of::<T>(), Arc::new(value));
    debug!("{:?} stored", std::any::type_name::<T>());
    Ok(())
  }
}

/// Type definition for RPC method callbacks
pub type RpcMethodCallback = Arc<
  dyn Fn(Option<&Value>, Arc<RpcServer>) -> Pin<Box<dyn Future<Output = Result<Value, anyhow::Error>> + Send>> + Send + Sync
>;

/// Registry for RPC methods that can be called by clients
#[derive(Clone)]
pub struct RpcMethodRegistry {
  /// Map of method names to their handler functions
  pub methods: HashMap<String, RpcMethodCallback>,
}

impl RpcMethodRegistry {
  /// Create a new empty RPC method registry
  pub fn new() -> Self {
    debug!("creating method registry");
    Self {
      methods: HashMap::new(),
    }
  }

  /// Register a new RPC method handler
  pub fn handle(&mut self, method: &str, callback: RpcMethodCallback) -> Result<(), anyhow::Error> {
    if self.methods.contains_key(method) {
      warn!("method '{}' already exists", method);
      return Err(anyhow!("Method {} already exists", method));
    }
    debug!("registered method: {}", method);
    self.methods.insert(method.to_string(), callback);
    Ok(())
  }

  /// Call a registered RPC method with the given message
  pub async fn call(&self, message: &RpcMessage, server: Arc<RpcServer>) -> Result<Value, anyhow::Error> {
    debug!("calling method: {}", message.method);
    let callback = match self.methods.get(&message.method) {
      Some(cb) => cb,
      None => {
        warn!("method not found: {}", message.method);
        return Err(anyhow!("Method {} not found", message.method));
      }
    };

    let result = callback(message.params.as_ref(), server).await;
    match &result {
      Ok(_) => debug!("method {} succeeded", message.method),
      Err(e) => warn!("method {} failed: {}", message.method, e),
    }
    result
  }
}
