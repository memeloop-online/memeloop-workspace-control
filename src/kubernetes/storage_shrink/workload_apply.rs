use std::{collections::BTreeSet, sync::OnceLock};

use serde::Deserialize;
use serde_json::{Map, Value, json};

const INVALID_FIELDS: &str = "workspace apply ownership is missing or unsupported";

pub(super) fn retained_workload_apply(current: &Value, stop: bool) -> Result<Value, String> {
    let managers = current["metadata"]["managedFields"]
        .as_array()
        .ok_or(INVALID_FIELDS)?;
    let mut owned = managers.iter().filter(|entry| {
        entry["manager"].as_str() == Some(super::super::FIELD_MANAGER)
            && entry["operation"].as_str() == Some("Apply")
            && entry["apiVersion"].as_str() == Some("apps/v1")
            && entry["subresource"].as_str().unwrap_or_default().is_empty()
            && entry["fieldsType"].as_str() == Some("FieldsV1")
    });
    let fields = &owned.next().ok_or(INVALID_FIELDS)?["fieldsV1"];
    if owned.next().is_some() {
        return Err(INVALID_FIELDS.to_owned());
    }
    validate_replica_ownership(managers)?;
    let mut apply = project_owned_fields(&current, fields, 0)?;
    let root = apply.as_object_mut().ok_or(INVALID_FIELDS)?;
    if root.keys().any(|key| key != "metadata" && key != "spec") {
        return Err(INVALID_FIELDS.to_owned());
    }
    root.insert("apiVersion".to_owned(), json!("apps/v1"));
    root.insert("kind".to_owned(), json!("StatefulSet"));
    let metadata = root
        .entry("metadata")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or(INVALID_FIELDS)?;
    if metadata.contains_key("managedFields") {
        return Err(INVALID_FIELDS.to_owned());
    }
    for field in ["name", "namespace", "uid", "resourceVersion"] {
        let value = current["metadata"][field]
            .as_str()
            .filter(|value| !value.is_empty())
            .ok_or(INVALID_FIELDS)?;
        metadata.insert(field.to_owned(), json!(value));
    }
    let spec = root
        .get_mut("spec")
        .and_then(Value::as_object_mut)
        .ok_or(INVALID_FIELDS)?;
    let policy = spec
        .entry("persistentVolumeClaimRetentionPolicy")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or(INVALID_FIELDS)?;
    for field in ["whenScaled", "whenDeleted"] {
        policy.insert(field.to_owned(), json!("Retain"));
    }
    if stop {
        spec.insert("replicas".to_owned(), json!(0));
    }
    Ok(apply)
}

fn validate_replica_ownership(managers: &[Value]) -> Result<(), String> {
    for entry in managers {
        let ours = entry["manager"].as_str() == Some(super::super::FIELD_MANAGER)
            && entry["operation"].as_str() == Some("Apply")
            && entry["apiVersion"].as_str() == Some("apps/v1")
            && entry["subresource"].as_str().unwrap_or_default().is_empty();
        if !ours
            && entry["fieldsV1"]["f:spec"]
                .as_object()
                .is_some_and(|fields| fields.contains_key("f:replicas") || fields.is_empty())
        {
            return Err(
                "workspace replicas have another field owner; refusing to override".to_owned(),
            );
        }
    }
    Ok(())
}

pub(super) fn project_owned_fields(
    current: &Value,
    fields: &Value,
    depth: usize,
) -> Result<Value, String> {
    project_fields_at(current, fields, depth, "")
}

#[derive(Deserialize)]
struct AtomicFields {
    objects: BTreeSet<String>,
    lists: BTreeSet<String>,
}

fn atomic_leaf(current: &Value, path: &str) -> Result<bool, String> {
    static FIELDS: OnceLock<Result<AtomicFields, String>> = OnceLock::new();
    let fields = FIELDS
        .get_or_init(|| {
            serde_json::from_str(include_str!("statefulset_atomic_fields.json"))
                .map_err(|_| INVALID_FIELDS.to_owned())
        })
        .as_ref()
        .map_err(Clone::clone)?;
    match current {
        Value::Object(object) => Ok(object.is_empty() || fields.objects.contains(path)),
        Value::Array(items) => Ok(items.is_empty() || fields.lists.contains(path)),
        _ => Ok(true),
    }
}

fn project_fields_at(
    current: &Value,
    fields: &Value,
    depth: usize,
    path: &str,
) -> Result<Value, String> {
    if depth > 64 {
        return Err(INVALID_FIELDS.to_owned());
    }
    let fields = fields.as_object().ok_or(INVALID_FIELDS)?;
    if fields.is_empty() {
        return atomic_leaf(current, path)?
            .then(|| current.clone())
            .ok_or_else(|| INVALID_FIELDS.to_owned());
    }
    if let Some(values) = current.as_array() {
        return project_list(values, fields, depth, path);
    }
    let source = current.as_object().ok_or(INVALID_FIELDS)?;
    let mut projected = Map::new();
    for (key, children) in fields {
        if key == "." && children.as_object().is_some_and(Map::is_empty) {
            continue;
        }
        let name = key.strip_prefix("f:").ok_or(INVALID_FIELDS)?;
        if let Some(value) = source.get(name) {
            let child_path = format!("{path}/{}", name.replace('~', "~0").replace('/', "~1"));
            projected.insert(
                name.to_owned(),
                project_fields_at(value, children, depth + 1, &child_path)?,
            );
        }
    }
    Ok(Value::Object(projected))
}

struct ListField<'fields> {
    selector: Value,
    children: &'fields Value,
    keyed: bool,
    matched: bool,
}

impl ListField<'_> {
    fn matches(&self, item: &Value) -> bool {
        if !self.keyed {
            return self.selector == *item;
        }
        self.selector
            .as_object()
            .zip(item.as_object())
            .is_some_and(|(keys, source)| {
                keys.iter()
                    .all(|(name, value)| source.get(name) == Some(value))
            })
    }

    fn project(&self, item: &Value, depth: usize, path: &str) -> Result<Value, String> {
        if !self.keyed {
            if !self.children.as_object().is_some_and(Map::is_empty) {
                return Err(INVALID_FIELDS.to_owned());
            }
            return Ok(item.clone());
        }
        let mut result = project_fields_at(item, self.children, depth + 1, &format!("{path}/[]"))?;
        let object = result.as_object_mut().ok_or(INVALID_FIELDS)?;
        for (name, value) in self.selector.as_object().ok_or(INVALID_FIELDS)? {
            object.insert(name.to_owned(), value.clone());
        }
        Ok(result)
    }
}

fn list_fields(fields: &Map<String, Value>) -> Result<Vec<ListField<'_>>, String> {
    let mut selected = Vec::new();
    for (key, children) in fields {
        if key == "." && children.as_object().is_some_and(Map::is_empty) {
            continue;
        }
        let (selector, keyed) = if let Some(selector) = key.strip_prefix("k:") {
            (selector, true)
        } else if let Some(selector) = key.strip_prefix("v:") {
            (selector, false)
        } else {
            return Err(INVALID_FIELDS.to_owned());
        };
        let selector: Value = serde_json::from_str(selector).map_err(|_| INVALID_FIELDS)?;
        if keyed && !selector.as_object().is_some_and(|keys| !keys.is_empty()) {
            return Err(INVALID_FIELDS.to_owned());
        }
        selected.push(ListField {
            selector,
            children,
            keyed,
            matched: false,
        });
    }
    Ok(selected)
}

fn project_list(
    current: &[Value],
    fields: &Map<String, Value>,
    depth: usize,
    path: &str,
) -> Result<Value, String> {
    let mut selected = list_fields(fields)?;
    let mut result = Vec::new();
    for item in current {
        let mut matching = selected.iter_mut().filter(|field| field.matches(item));
        if let Some(field) = matching.next() {
            if matching.next().is_some() || field.matched {
                return Err(INVALID_FIELDS.to_owned());
            }
            field.matched = true;
            result.push(field.project(item, depth, path)?);
        }
    }
    if selected.iter().any(|field| !field.matched) {
        return Err(INVALID_FIELDS.to_owned());
    }
    Ok(Value::Array(result))
}
