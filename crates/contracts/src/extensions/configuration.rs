//! Adapter to the existing caller-scoped configuration service.
//!
//! `amaterasu.configuration/1` owns schema validation, defaults, revisions,
//! migrations and persistence. This contract only ties an extension's option
//! namespace to its exact provider principal and formats existing operations.

use super::ExtensionVersion;
use crate::valid_name;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

pub const CONFIGURATION_SERVICE: &str = "amaterasu.configuration/1";
// The runtime broker rejects serialized request payloads above this size.
const MAX_DECLARATION_BYTES: usize = 16_384;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionConfigurationRef {
    pub version: ExtensionVersion,
    /// Must equal both the approved component identity and authenticated
    /// configuration caller. The service never accepts a caller-supplied path.
    pub caller_component: String,
    pub namespace: String,
}

/// A display category for an existing caller-owned configuration document.
/// This is UI metadata, not a new settings store or a permission grant. The
/// management API must read the registered schema and values through its own
/// authenticated, scope-checked configuration route before rendering fields.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExtensionSettingsCategory {
    General,
    Capture,
    Encoding,
    Audio,
    Input,
    Network,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExtensionSettingsContribution {
    pub configuration: ExtensionConfigurationRef,
    pub category: ExtensionSettingsCategory,
    /// Plain-text heading only. Clients must render it as text, never markup.
    pub label: String,
}

impl ExtensionSettingsContribution {
    pub fn validate_for(&self, component_id: &str) -> Result<(), &'static str> {
        self.configuration.validate_for(component_id)?;
        if self.label.is_empty()
            || self.label.chars().count() > 80
            || self.label.chars().any(char::is_control)
        {
            return Err("extension settings label is invalid");
        }
        Ok(())
    }
}

impl ExtensionConfigurationRef {
    pub fn validate_for(&self, component_id: &str) -> Result<(), &'static str> {
        if !self.version.is_supported()
            || !valid_name(&self.caller_component)
            || self.caller_component != component_id
            || self.namespace != component_id
        {
            return Err("extension configuration identity or version is invalid");
        }
        Ok(())
    }

    /// Existing `register` operation. The configuration module, not this
    /// adapter, validates field types, bounds, defaults and conflicts.
    pub fn register_payload(&self, fields: Map<String, Value>) -> Result<Value, &'static str> {
        self.validate_for(&self.caller_component)?;
        bounded(json!({"action":"register","fields":fields}))
    }

    pub fn get_payload(&self) -> Result<Value, &'static str> {
        self.validate_for(&self.caller_component)?;
        Ok(json!({"action":"get"}))
    }

    /// Existing revisioned `update` operation. No implicit retry is safe on a
    /// conflict or uncertain result; callers must read the current revision.
    pub fn update_payload(
        &self,
        values: Map<String, Value>,
        expected_revision: u64,
    ) -> Result<Value, &'static str> {
        self.validate_for(&self.caller_component)?;
        bounded(json!({
            "action":"update",
            "values":values,
            "expected_revision":expected_revision,
        }))
    }
}

fn bounded(payload: Value) -> Result<Value, &'static str> {
    if serde_json::to_vec(&payload)
        .map_err(|_| "invalid extension configuration encoding")?
        .len()
        > MAX_DECLARATION_BYTES
    {
        return Err("extension configuration declaration is too large");
    }
    Ok(payload)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn configuration() -> ExtensionConfigurationRef {
        ExtensionConfigurationRef {
            version: ExtensionVersion::CURRENT,
            caller_component: "org.example.capture".into(),
            namespace: "org.example.capture".into(),
        }
    }

    #[test]
    fn adapter_uses_existing_caller_scoped_revisioned_operations() {
        let config = configuration();
        let fields = Map::from_iter([(
            "quality".into(),
            json!({"type":"integer","default":5,"minimum":1,"maximum":10}),
        )]);
        assert_eq!(
            config.register_payload(fields).unwrap()["action"],
            "register"
        );
        assert_eq!(config.get_payload().unwrap(), json!({"action":"get"}));
        let update = config
            .update_payload(Map::from_iter([("quality".into(), json!(7))]), 11)
            .unwrap();
        assert_eq!(update["expected_revision"], 11);
        assert_eq!(update["values"]["quality"], 7);
    }

    #[test]
    fn caller_cannot_claim_another_providers_namespace() {
        let mut config = configuration();
        config.namespace = "org.example.other".into();
        assert!(config.register_payload(Map::new()).is_err());
        config.namespace = config.caller_component.clone();
        assert!(config.validate_for("org.example.other").is_err());
        config.version.major += 1;
        assert!(config.get_payload().is_err());
    }

    #[test]
    fn declaration_must_fit_the_runtime_broker_request() {
        let config = configuration();
        let too_large = Map::from_iter([("description".into(), json!("x".repeat(20_000)))]);
        assert!(config.register_payload(too_large).is_err());
        let bounded = Map::from_iter([("description".into(), json!("x".repeat(100)))]);
        assert!(config.register_payload(bounded).is_ok());
    }

    #[test]
    fn category_reference_cannot_take_another_components_settings() {
        let mut contribution = ExtensionSettingsContribution {
            configuration: configuration(),
            category: ExtensionSettingsCategory::Encoding,
            label: "Encoder quality".into(),
        };
        assert!(contribution.validate_for("org.example.capture").is_ok());
        assert!(contribution.validate_for("org.example.other").is_err());
        contribution.label = "<script>\n".into();
        assert!(contribution.validate_for("org.example.capture").is_err());
        contribution.label = "x".repeat(81);
        assert!(contribution.validate_for("org.example.capture").is_err());
    }
}
