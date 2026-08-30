use std::collections::BTreeMap;

use sha2::{Digest, Sha256};

use crate::{
    plugins::{PluginManifest, validate_plugin_content},
    storage::PluginAssetBlob,
};

pub(crate) const MAX_PACKAGE_BYTES: usize = 80 * 1024 * 1024;
const MAGIC: &[u8; 8] = b"MWCPKG01";
const MAX_MANIFEST_BYTES: usize = 1024 * 1024;
const MAX_COMPONENT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug)]
pub(crate) struct PreparedPluginPackage {
    pub manifest: PluginManifest,
    pub manifest_json: String,
    pub component: Option<Vec<u8>>,
    pub assets: Vec<PluginAssetBlob>,
    pub digest: String,
    pub size_bytes: u64,
    pub declared_contributions: Vec<String>,
}

impl PreparedPluginPackage {
    pub(crate) fn prepare(
        manifest: Vec<u8>,
        component: Option<Vec<u8>>,
        assets: Vec<(String, String, Vec<u8>)>,
    ) -> Result<Self, &'static str> {
        if manifest.len() > MAX_MANIFEST_BYTES
            || component
                .as_ref()
                .is_some_and(|value| value.len() > MAX_COMPONENT_BYTES)
        {
            return Err("plugin_package_invalid");
        }
        let asset_map = assets
            .iter()
            .map(|(path, media_type, content)| {
                (path.clone(), (media_type.clone(), content.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let validated = validate_plugin_content(&manifest, component.as_deref(), &asset_map)
            .map_err(|_| "plugin_package_invalid")?;
        let manifest_json = String::from_utf8(manifest).map_err(|_| "plugin_package_invalid")?;
        let mut hasher = Sha256::new();
        hash_part(&mut hasher, manifest_json.as_bytes());
        hash_part(&mut hasher, component.as_deref().unwrap_or_default());
        let mut blobs = Vec::with_capacity(assets.len());
        for (path, media_type, content) in assets {
            hash_part(&mut hasher, path.as_bytes());
            hash_part(&mut hasher, media_type.as_bytes());
            hash_part(&mut hasher, &content);
            blobs.push(PluginAssetBlob {
                path,
                media_type,
                digest: format!("{:x}", Sha256::digest(&content)),
                content,
            });
        }
        blobs.sort_by(|left, right| left.path.cmp(&right.path));
        let size_bytes = manifest_json.len()
            + component.as_ref().map_or(0, Vec::len)
            + blobs.iter().map(|asset| asset.content.len()).sum::<usize>();
        let declared_contributions = contributions(&validated.manifest);
        Ok(Self {
            manifest: validated.manifest,
            manifest_json,
            component,
            assets: blobs,
            digest: format!("{:x}", hasher.finalize()),
            size_bytes: size_bytes as u64,
            declared_contributions,
        })
    }
}

pub(crate) fn decode_bundle(bytes: &[u8]) -> Result<PreparedPluginPackage, &'static str> {
    if bytes.len() > MAX_PACKAGE_BYTES || bytes.get(..MAGIC.len()) != Some(MAGIC) {
        return Err("plugin_package_invalid");
    }
    let mut cursor = Cursor {
        bytes,
        offset: MAGIC.len(),
    };
    let manifest_len = cursor.u32()? as usize;
    let component_len = cursor.u32()? as usize;
    let asset_count = cursor.u16()? as usize;
    if manifest_len > MAX_MANIFEST_BYTES || component_len > MAX_COMPONENT_BYTES || asset_count > 64
    {
        return Err("plugin_package_invalid");
    }
    let manifest = cursor.take(manifest_len)?.to_vec();
    let component = (component_len != 0)
        .then(|| cursor.take(component_len).map(Vec::from))
        .transpose()?;
    let mut assets = Vec::with_capacity(asset_count);
    for _ in 0..asset_count {
        let path_len = cursor.u16()? as usize;
        let media_len = cursor.u16()? as usize;
        let content_len = cursor.u32()? as usize;
        let path = std::str::from_utf8(cursor.take(path_len)?)
            .map_err(|_| "plugin_package_invalid")?
            .to_owned();
        let media = std::str::from_utf8(cursor.take(media_len)?)
            .map_err(|_| "plugin_package_invalid")?
            .to_owned();
        assets.push((path, media, cursor.take(content_len)?.to_vec()));
    }
    if cursor.offset != bytes.len() {
        return Err("plugin_package_invalid");
    }
    PreparedPluginPackage::prepare(manifest, component, assets)
}

fn contributions(manifest: &PluginManifest) -> Vec<String> {
    [
        (manifest.workspace_create_policy, "workspace_create_policy"),
        (manifest.configuration.is_some(), "configuration"),
        (!manifest.ui_surfaces.is_empty(), "ui_surfaces"),
        (!manifest.api_routes.is_empty(), "api_routes"),
        (!manifest.api_middleware.is_empty(), "api_middleware"),
    ]
    .into_iter()
    .filter(|(present, _)| *present)
    .map(|(_, name)| name.to_owned())
    .collect()
}

fn hash_part(hasher: &mut Sha256, value: &[u8]) {
    hasher.update((value.len() as u64).to_be_bytes());
    hasher.update(value);
}

struct Cursor<'a> {
    bytes: &'a [u8],
    offset: usize,
}

impl<'a> Cursor<'a> {
    fn take(&mut self, length: usize) -> Result<&'a [u8], &'static str> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or("plugin_package_invalid")?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or("plugin_package_invalid")?;
        self.offset = end;
        Ok(value)
    }
    fn u16(&mut self) -> Result<u16, &'static str> {
        Ok(u16::from_be_bytes(
            self.take(2)?
                .try_into()
                .map_err(|_| "plugin_package_invalid")?,
        ))
    }
    fn u32(&mut self) -> Result<u32, &'static str> {
        Ok(u32::from_be_bytes(
            self.take(4)?
                .try_into()
                .map_err(|_| "plugin_package_invalid")?,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_pathological_bundle_lengths() {
        let mut bytes = MAGIC.to_vec();
        bytes.extend_from_slice(&u32::MAX.to_be_bytes());
        bytes.extend_from_slice(&0_u32.to_be_bytes());
        bytes.extend_from_slice(&0_u16.to_be_bytes());
        assert_eq!(decode_bundle(&bytes).unwrap_err(), "plugin_package_invalid");
    }
}
