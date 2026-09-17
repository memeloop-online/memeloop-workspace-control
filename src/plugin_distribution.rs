mod bundle;
mod download;

pub(crate) use bundle::{MAX_PACKAGE_BYTES, PreparedPluginPackage, decode_bundle};
pub(crate) use download::{download_github_release, download_https, sanitized_source_ref};
