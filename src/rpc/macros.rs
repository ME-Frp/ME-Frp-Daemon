/// ```
/// rpc_handler!(registry, {
///   "hello" => |_params, server| {
///     info!("Hello, world!");
///     Ok(Value::String("Hello, world!".to_string()))
///   },
///   
///   "add" => |params: Value, _server| {
///     let a = params["a"].as_i64().unwrap_or(0);
///     let b = params["b"].as_i64().unwrap_or(0);
///     Ok(Value::Number((a + b).into()))
///   }
/// });
/// ```
#[macro_export]
macro_rules! rpc_handler {
    ($registry:expr, { $( $method:expr => |$param:ident $(: $param_type:ty)?, $server:ident| $handler_body:expr ),* $(,)? }) => {
        {
            use std::sync::Arc;
            use std::pin::Pin;
            use std::future::Future;
            use serde_json::Value;
            use crate::rpc::RpcServer;

            $(
                $registry.handle($method, Arc::new(|params: Option<&Value>, $server: Arc<RpcServer>| -> Pin<Box<dyn Future<Output = Result<Value, anyhow::Error>> + Send>> {
                    let $param $(: $param_type)? = params.cloned().unwrap_or(Value::Null);
                    Box::pin(async move {
                        $handler_body.await
                    })
                }))?;
            )*
        }
    };

    ($registry:expr, { $( $method:expr => sync |$param:ident $(: $param_type:ty)?, $server:ident| $handler_body:expr ),* $(,)? }) => {
        {
            use std::sync::Arc;
            use std::pin::Pin;
            use std::future::Future;
            use serde_json::Value;
            use crate::rpc::RpcServer;

            $(
                $registry.handle($method, Arc::new(|params: Option<&Value>, $server: Arc<RpcServer>| -> Pin<Box<dyn Future<Output = Result<Value, anyhow::Error>> + Send>> {
                    let $param $(: $param_type)? = params.cloned().unwrap_or(Value::Null);
                    Box::pin(async move {
                        $handler_body
                    })
                }))?;
            )*
        }
    };
}

/// ```
/// let handler = rpc_fn!(|params: Value, server| {
///     let name = params["name"].as_str().unwrap_or("Guest");
///     // Access RPC server instance methods via server parameter, such as broadcast
///     Ok(Value::String(format!("Hello, {}!", name)))
/// });
/// registry.handle("greet", handler)?;
/// ```
#[macro_export]
macro_rules! rpc_fn {
    (|$params:ident $(: $param_type:ty)?, $server:ident| $body:block) => {
        {
            use std::sync::Arc;
            use std::pin::Pin;
            use std::future::Future;
            use serde_json::Value;
            use crate::rpc::RpcServer;

            Arc::new(|params_opt: Option<&Value>, $server: Arc<RpcServer>| -> Pin<Box<dyn Future<Output = Result<Value, anyhow::Error>> + Send>> {
                let $params $(: $param_type)? = params_opt.cloned().unwrap_or(Value::Null);
                Box::pin(async move { $body })
            })
        }
    };
}
