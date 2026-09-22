//! Stable edition/capability surface shared by both editions.
//!
//! This module is the *only* place the community code learns that optional
//! commercial features exist. It describes capability metadata and licence
//! state; it contains no implementation. A commercial composition registers
//! its own capabilities here and keeps its implementation private.
//!
//! Keeping this boundary generic avoids scattered `if is_pro` checks: callers
//! ask the registry whether a capability is available and render accordingly.

use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Edition {
    Community,
    Commercial,
}

/// Product-level description of an optional capability.
///
/// Deliberately limited to identity and availability. It must never describe
/// how a capability is implemented, scanned, stored or licensed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct FeatureCapability {
    pub id: String,
    pub name: String,
    pub edition: Edition,
    pub available: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LicenseState {
    Unlicensed,
    Licensed,
    Expired,
    Invalid,
}

/// Registry of optional capabilities. The community build starts empty; a
/// commercial composition populates it before the app is built.
#[derive(Debug, Default, Clone)]
pub struct FeatureRegistry {
    capabilities: Vec<FeatureCapability>,
}

impl FeatureRegistry {
    /// An empty registry: the community edition.
    pub fn community() -> Self {
        Self::default()
    }

    pub fn register(&mut self, capability: FeatureCapability) {
        if let Some(existing) = self
            .capabilities
            .iter_mut()
            .find(|entry| entry.id == capability.id)
        {
            *existing = capability;
            return;
        }
        self.capabilities.push(capability);
    }

    pub fn capabilities(&self) -> &[FeatureCapability] {
        &self.capabilities
    }

    pub fn is_available(&self, id: &str) -> bool {
        self.capabilities
            .iter()
            .any(|entry| entry.id == id && entry.available)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn community_registry_exposes_no_capabilities() {
        let registry = FeatureRegistry::community();
        assert!(registry.capabilities().is_empty());
        assert!(!registry.is_available("anything"));
    }

    #[test]
    fn registering_a_capability_replaces_rather_than_duplicates() {
        let mut registry = FeatureRegistry::community();
        let capability = |available| FeatureCapability {
            id: "example".into(),
            name: "Example".into(),
            edition: Edition::Commercial,
            available,
        };
        registry.register(capability(false));
        registry.register(capability(true));
        assert_eq!(registry.capabilities().len(), 1);
        assert!(registry.is_available("example"));
    }
}
