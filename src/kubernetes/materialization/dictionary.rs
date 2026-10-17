use std::collections::BTreeMap;

use k8s_openapi::api::core::v1::ConfigMap;
use serde::Serialize;
use serde_json::Value;

use super::MaterializationError;

const MIN_BYTES: usize = 64 * 1024;
const MANIFEST: &str = "workspace-files.json";

/// Lossless text dictionaries use jq, already required by Image Contract v1.
/// Literal JSON newline escapes expose repeated instruction paragraphs without parsing or
/// reformatting the original file. Rejoining chunks preserves every input byte.
#[derive(Serialize)]
struct Dictionary<'a> {
    version: u8,
    chunks: Vec<&'a str>,
    order: Vec<usize>,
}

fn encode(value: &str) -> Result<Option<String>, MaterializationError> {
    if value.len() < MIN_BYTES {
        return Ok(None);
    }
    let mut indices = BTreeMap::new();
    let mut dictionary = Dictionary {
        version: 1,
        chunks: Vec::new(),
        order: Vec::new(),
    };
    for chunk in value
        .split_inclusive('\n')
        .flat_map(|line| line.split_inclusive("\\n"))
    {
        let next = indices.len();
        let index = *indices.entry(chunk).or_insert_with(|| {
            dictionary.chunks.push(chunk);
            next
        });
        dictionary.order.push(index);
    }
    let encoded = serde_json::to_string(&dictionary)?;
    // ConfigMap text is itself inside a JSON string. Count both escaping layers before
    // opting in; an encoding that fails to save at least 25% stays entirely unchanged.
    if serde_json::to_vec(&encoded)?.len() * 4 < serde_json::to_vec(value)?.len() * 3 {
        Ok(Some(encoded))
    } else {
        Ok(None)
    }
}

pub(super) fn compress(original: &ConfigMap) -> Result<ConfigMap, MaterializationError> {
    let data = original
        .data
        .as_ref()
        .ok_or(MaterializationError::InvalidFileManifest)?;
    let manifest = data
        .get(MANIFEST)
        .ok_or(MaterializationError::InvalidFileManifest)?;
    let mut manifest: Value = serde_json::from_str(manifest)?;
    let files = manifest["files"]
        .as_array_mut()
        .ok_or(MaterializationError::InvalidFileManifest)?;
    let mut result = original.clone();
    let mut changed = false;
    for entry in files {
        // Environment values, SSH keys and secrets retain their exact original transport.
        // Binary ConfigMap entries also remain unchanged.
        if entry["object"] != "config_map" || entry["kind"] != "config_file" {
            continue;
        }
        if entry["encoding"] == "dictionary_v1" {
            continue;
        }
        if !entry["encoding"].is_null() {
            return Err(MaterializationError::InvalidFileManifest);
        }
        let key = entry["data_key"]
            .as_str()
            .ok_or(MaterializationError::InvalidFileManifest)?;
        let Some(value) = data.get(key) else {
            continue;
        };
        let Some(encoded) = encode(value)? else {
            continue;
        };
        result
            .data
            .as_mut()
            .unwrap()
            .insert(key.to_owned(), encoded);
        entry["encoding"] = Value::from("dictionary_v1");
        entry["decoded_bytes"] = Value::from(value.len());
        changed = true;
    }
    if changed {
        result
            .data
            .as_mut()
            .unwrap()
            .insert(MANIFEST.to_owned(), serde_json::to_string(&manifest)?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use k8s_openapi::ByteString;
    use serde_json::json;

    fn repeated_text() -> String {
        concat!(
            "中文🦀 ",
            r#"\"quotes\""#,
            "\r\n repeated instruction\\nnext paragraph\n"
        )
        .repeat(2048)
    }

    fn config(value: &str) -> ConfigMap {
        ConfigMap {
            data: Some(BTreeMap::from([
                ("file-0000".to_owned(), value.to_owned()),
                ("env-0001".to_owned(), value.to_owned()),
                (MANIFEST.to_owned(), json!({"version":1,"files":[
                    {"object":"config_map","kind":"config_file","data_key":"file-0000",
                     "target":"/home/user/config","mode":416,"owner":"user","group":"user"},
                    {"object":"secret","kind":"secret_file","data_key":"secret-key","target":"/home/user/auth"},
                    {"object":"config_map","kind":"config_file","data_key":"binary-key","target":"/home/user/binary"}
                ]}).to_string()),
            ])),
            binary_data: Some(BTreeMap::from([("binary-key".to_owned(), ByteString(vec![255;MIN_BYTES]))])),
            ..ConfigMap::default()
        }
    }

    #[test]
    fn dictionary_preserves_exact_utf8_bytes_metadata_and_non_file_values() {
        let text = repeated_text();
        let original = config(&text);
        let packed = compress(&original).unwrap();
        let packed_data = packed.data.as_ref().unwrap();
        let payload: Value = serde_json::from_str(&packed_data["file-0000"]).unwrap();
        let decoded = payload["order"]
            .as_array()
            .unwrap()
            .iter()
            .map(|index| {
                payload["chunks"][index.as_u64().unwrap() as usize]
                    .as_str()
                    .unwrap()
            })
            .collect::<String>();
        assert_eq!(decoded.as_bytes(), text.as_bytes());
        assert!(
            serde_json::to_vec(&packed_data["file-0000"]).unwrap().len() * 4
                < serde_json::to_vec(&text).unwrap().len() * 3
        );
        let manifest: Value = serde_json::from_str(&packed_data[MANIFEST]).unwrap();
        assert_eq!(manifest["files"][0]["encoding"], "dictionary_v1");
        assert_eq!(manifest["files"][0]["decoded_bytes"], text.len());
        assert_eq!(manifest["files"][0]["mode"], 416);
        assert_eq!(manifest["files"][0]["owner"], "user");
        assert!(manifest["files"][1].get("encoding").is_none());
        assert!(manifest["files"][2].get("encoding").is_none());
        assert_eq!(packed_data["env-0001"], text);
        assert_eq!(packed.binary_data, original.binary_data);
        assert_eq!(original.data.as_ref().unwrap()["file-0000"], text);
        assert_eq!(
            compress(&packed).unwrap(),
            packed,
            "packing must be idempotent"
        );
    }

    #[test]
    fn small_or_unhelpful_dictionaries_remain_byte_for_byte_unchanged() {
        for text in ["small\n".to_owned(), "x".repeat(MIN_BYTES + 1)] {
            let original = config(&text);
            assert_eq!(compress(&original).unwrap(), original);
        }
    }

    #[test]
    fn malformed_manifest_or_encoding_is_rejected() {
        for manifest in [
            "{",
            r#"{"files":null}"#,
            r#"{"files":[{"object":"config_map","kind":"config_file","data_key":"file-0000","encoding":"unknown"}]}"#,
        ] {
            let mut original = config(&repeated_text());
            original
                .data
                .as_mut()
                .unwrap()
                .insert(MANIFEST.to_owned(), manifest.to_owned());
            assert!(compress(&original).is_err());
        }
    }
}
