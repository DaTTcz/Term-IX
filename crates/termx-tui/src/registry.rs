use std::collections::HashMap;
use std::sync::Arc;

use termx_core::ProtocolModule;

/// Registr dostupnych protokolovych modulu. `main.rs` sem pri startu
/// zaregistruje kazdy modul, ktery je do aplikace prilinkovany
/// (v MVP jen SSH) - TUI pak s protokoly pracuje jen pres tento registr.
#[derive(Default)]
pub struct ModuleRegistry {
    modules: HashMap<&'static str, Arc<dyn ProtocolModule>>,
}

impl ModuleRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, module: Arc<dyn ProtocolModule>) {
        self.modules.insert(module.protocol_key(), module);
    }

    pub fn get(&self, key: &str) -> Option<&Arc<dyn ProtocolModule>> {
        self.modules.get(key)
    }

    pub fn is_supported(&self, key: &str) -> bool {
        self.modules.contains_key(key)
    }
}
