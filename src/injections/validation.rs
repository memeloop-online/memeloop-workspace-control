use base64::{Engine, engine::general_purpose::STANDARD};
use uuid::Uuid;

use super::{InjectionError, InjectionItem, InjectionKind, InjectionValue};

pub fn validate_injection_item(item: &InjectionItem) -> Result<(), InjectionError> {
    validate_injection_key(&item.key)?;
    if item.file_mode.is_some_and(|mode| mode > 0o777) {
        return Err(InjectionError::InvalidFileMode);
    }
    for name in [item.owner.as_deref(), item.group.as_deref()]
        .into_iter()
        .flatten()
    {
        if name.is_empty()
            || name.len() > 64
            || name.starts_with('-')
            || !name
                .chars()
                .all(|character| character.is_ascii_alphanumeric() || "_-".contains(character))
        {
            return Err(InjectionError::InvalidOwnerOrGroup);
        }
    }
    if let Some(selector) = item.template_selector.as_deref()
        && selector != "*"
        && Uuid::parse_str(selector).is_err()
    {
        return Err(InjectionError::InvalidTemplateSelector);
    }
    if item.labels.iter().any(|(key, value)| {
        key.is_empty()
            || key.len() > 128
            || value.is_empty()
            || value.len() > 512
            || key.chars().any(char::is_whitespace)
    }) {
        return Err(InjectionError::InvalidLabelSelector);
    }
    match item.kind {
        InjectionKind::EnvironmentVariable => {
            validate_environment_target(&item.target)?;
            validate_environment_value(&item.value)?;
        }
        InjectionKind::SecretFile | InjectionKind::ConfigFile => {
            validate_file_target(&item.target)?
        }
        InjectionKind::SshPublicKey => {
            validate_file_target(&item.target)?;
            let InjectionValue::Utf8(value) = &item.value else {
                return Err(InjectionError::InvalidSshPublicKey);
            };
            ssh_key::PublicKey::from_openssh(value.trim())
                .map_err(|_| InjectionError::InvalidSshPublicKey)?;
        }
    }
    if let InjectionValue::Base64(value) = &item.value {
        STANDARD
            .decode(value)
            .map_err(|_| InjectionError::InvalidBase64)?;
    }
    Ok(())
}

pub fn validate_injection_key(key: &str) -> Result<(), InjectionError> {
    if key.trim() != key
        || key.is_empty()
        || key.chars().count() > 128
        || !key
            .chars()
            .all(|character| character.is_alphanumeric() || " ._-".contains(character))
    {
        return Err(InjectionError::InvalidKey);
    }
    Ok(())
}

fn validate_environment_target(target: &str) -> Result<(), InjectionError> {
    let mut chars = target.chars();
    if chars
        .next()
        .is_none_or(|first| first != '_' && !first.is_ascii_alphabetic())
        || !chars.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        return Err(InjectionError::InvalidTarget);
    }
    Ok(())
}

fn validate_environment_value(value: &InjectionValue) -> Result<(), InjectionError> {
    let decoded;
    let bytes = match value {
        InjectionValue::Utf8(value) => value.as_bytes(),
        InjectionValue::Base64(value) => {
            decoded = STANDARD
                .decode(value)
                .map_err(|_| InjectionError::InvalidBase64)?;
            decoded.as_slice()
        }
    };
    let value = std::str::from_utf8(bytes).map_err(|_| InjectionError::InvalidEnvironmentValue)?;
    if value.chars().any(char::is_control) {
        return Err(InjectionError::InvalidEnvironmentValue);
    }
    Ok(())
}

fn validate_file_target(target: &str) -> Result<(), InjectionError> {
    if !target.starts_with('/')
        || target.contains('\0')
        || target.split('/').any(|component| component == "..")
    {
        return Err(InjectionError::InvalidTarget);
    }
    Ok(())
}
