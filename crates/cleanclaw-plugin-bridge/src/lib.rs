//! `cleanclaw-plugin-bridge` — OpenClaw-style plugin shim.
//!
//!
//! The original Go pipeline shipped a Node.js shim that loaded
//! an OpenClaw-style plugin and translated its `register(api)`
//! calls into the CleanClaw JSON-RPC protocol. The Rust pipeline
//! makes the shim a no-op for already-Rust plugins: every
//! `cleanclaw-plugins/*` crate speaks the same JSON-RPC protocol
//! directly, so no bridge is needed.
//!
//! ## When the bridge is still useful
//!
//! The bridge remains relevant for two narrow cases:
//!
//! 1. **JS / TS plugins** that need to run on the same host
//!    without compiling. The `openclaw-proxy` Node.js shim still
//!    works — point its `command:` field in `plugin.json` at
//!    `node /usr/local/lib/cleanclaw/openclaw-proxy.js`.
//! 2. **Legacy OpenClaw plugins** that depend on the OpenClaw
//!    `register(api)` API. Forward `registerTool` / `registerHook`
//!    calls to the same JSON-RPC surface this crate exposes.
//!
//! ## The native path
//!
//! For first-party plugins, prefer writing a Rust crate that
//! implements [`cleanclaw_plugin_runtime::Plugin`]. See
//! `cleanclaw-plugins/openclaw-demo/` for a working example.

use cleanclaw_plugin_runtime::{Plugin, PluginError};
use serde_json::Value;

/// A bridge plugin that delegates to an inner `Plugin`
/// implementation. Currently this is just `Deref` sugar; the
/// real bridge protocol (TS → JSON-RPC translation) lives in
/// the Node.js `openclaw-proxy.js` shim.
pub struct BridgePlugin<P: Plugin> {
    inner: P,
}

impl<P: Plugin> BridgePlugin<P> {
    pub fn new(inner: P) -> Self {
        Self { inner }
    }
}

#[async_trait::async_trait]
impl<P: Plugin + Send + Sync> Plugin for BridgePlugin<P> {
    fn id(&self) -> &str {
        self.inner.id()
    }

    async fn initialize(&self, params: Value) -> Result<Value, PluginError> {
        self.inner.initialize(params).await
    }

    async fn tool_list(&self) -> Result<Vec<cleanclaw_plugin_runtime::ToolDef>, PluginError> {
        self.inner.tool_list().await
    }

    async fn tool_execute(
        &self,
        name: &str,
        args: Value,
    ) -> Result<cleanclaw_plugin_runtime::ToolResult, PluginError> {
        self.inner.tool_execute(name, args).await
    }

    async fn hook_register(
        &self,
    ) -> Result<cleanclaw_plugin_runtime::HookRegistration, PluginError> {
        self.inner.hook_register().await
    }

    async fn hook_fire(&self, params: Value) -> Result<(), PluginError> {
        self.inner.hook_fire(params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cleanclaw_plugin_runtime::ToolDef;
    

    struct StubPlugin;

    #[async_trait::async_trait]
    impl Plugin for StubPlugin {
        fn id(&self) -> &str {
            "stub"
        }
        async fn tool_list(&self) -> Result<Vec<ToolDef>, PluginError> {
            Ok(vec![ToolDef {
                name: "stub".into(),
                description: "stub".into(),
                parameters: Value::Null,
                source: "plugin".into(),
            }])
        }
    }

    #[test]
    fn bridge_passthroughs() {
        let p = BridgePlugin::new(StubPlugin);
        assert_eq!(p.id(), "stub");
    }
}
