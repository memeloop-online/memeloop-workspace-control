use serde_json::Value;
use sha2::{Digest, Sha256};

use super::PluginError;

const MAX_SCHEMA_BYTES: usize = 256 * 1024;
const MAX_INSTANCE_BYTES: usize = 1024 * 1024;
const MAX_DEPTH: usize = 32;
const MAX_NODES: usize = 4_096;

#[derive(Clone)]
pub(super) struct ConfigurationSchema {
    validator: jsonschema::Validator,
    digest: String,
}

impl ConfigurationSchema {
    pub(super) fn compile(schema: &Value) -> Result<Self, PluginError> {
        if schema.get("type").and_then(Value::as_str) != Some("object") {
            return Err(PluginError::invalid(
                "configuration schema must have an object root",
            ));
        }
        if serde_json::to_vec(schema)
            .map_err(|_| PluginError::InvalidConfiguration)?
            .len()
            > MAX_SCHEMA_BYTES
        {
            return Err(PluginError::invalid("configuration schema exceeds 256 KiB"));
        }
        let mut nodes = 0;
        inspect(schema, 0, &mut nodes)?;
        let validator = jsonschema::draft202012::options()
            .build(schema)
            .map_err(|_| PluginError::invalid("configuration schema is invalid"))?;
        let canonical = canonical_json(schema);
        let encoded =
            serde_json::to_vec(&canonical).map_err(|_| PluginError::InvalidConfiguration)?;
        Ok(Self {
            validator,
            digest: format!("{:x}", Sha256::digest(encoded)),
        })
    }

    pub(super) fn validate(&self, value: &Value) -> Result<(), PluginError> {
        if serde_json::to_vec(value)
            .map_err(|_| PluginError::InvalidConfiguration)?
            .len()
            > MAX_INSTANCE_BYTES
        {
            return Err(PluginError::InvalidConfiguration);
        }
        // Validator errors can embed rejected values, so never propagate them.
        self.validator
            .validate(value)
            .map_err(|_| PluginError::InvalidConfiguration)
    }

    pub(super) fn digest(&self) -> &str {
        &self.digest
    }
}

fn canonical_json(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_json).collect()),
        Value::Object(values) => {
            let mut entries: Vec<_> = values.iter().collect();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            Value::Object(
                entries
                    .into_iter()
                    .map(|(key, value)| (key.clone(), canonical_json(value)))
                    .collect(),
            )
        }
        value => value.clone(),
    }
}

fn inspect(value: &Value, depth: usize, nodes: &mut usize) -> Result<(), PluginError> {
    *nodes = nodes.saturating_add(1);
    if depth > MAX_DEPTH || *nodes > MAX_NODES {
        return Err(PluginError::invalid(
            "configuration schema exceeds complexity limits",
        ));
    }
    match value {
        Value::Array(values) => {
            for value in values {
                inspect(value, depth + 1, nodes)?;
            }
        }
        Value::Object(object) => {
            if object.get("writeOnly").and_then(Value::as_bool) == Some(true) {
                return Err(PluginError::invalid(
                    "configuration cannot contain write-only fields",
                ));
            }
            if let Some(reference) = object.get("$ref").and_then(Value::as_str)
                && !reference.starts_with("#/")
            {
                return Err(PluginError::invalid(
                    "configuration schema references must be local",
                ));
            }
            if let Some(properties) = object.get("properties").and_then(Value::as_object) {
                for key in properties.keys() {
                    let normalized = key.to_ascii_lowercase().replace(['-', ' '], "_");
                    if [
                        "secret",
                        "password",
                        "token",
                        "credential",
                        "private_key",
                        "api_key",
                    ]
                    .iter()
                    .any(|needle| normalized.contains(needle))
                    {
                        return Err(PluginError::invalid(
                            "credentials must use the encrypted injection subsystem",
                        ));
                    }
                }
            }
            for child in object.values() {
                inspect(child, depth + 1, nodes)?;
            }
        }
        _ => {}
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn rejects_secret_shaped_configuration() {
        assert!(
            ConfigurationSchema::compile(&json!({
                "type":"object",
                "properties":{"api_token":{"type":"string"}}
            }))
            .is_err()
        );
    }

    #[test]
    fn validates_non_sensitive_values() {
        let schema = ConfigurationSchema::compile(&json!({
            "type":"object",
            "additionalProperties":false,
            "required":["max_workspaces"],
            "properties":{"max_workspaces":{"type":"integer","minimum":1}}
        }))
        .unwrap();
        assert!(schema.validate(&json!({"max_workspaces":2})).is_ok());
        assert!(schema.validate(&json!({"max_workspaces":0})).is_err());
    }
}
