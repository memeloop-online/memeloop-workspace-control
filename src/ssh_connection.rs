//! One naming contract for SSH configuration, host-key pins and key comments.

pub fn alias(name: &str, short_id: &str) -> String {
    let mut slug = String::new();
    for character in name.chars().take(48) {
        if character.is_alphanumeric() || character == '-' || character == '_' {
            slug.push(character);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug = slug.trim_matches('-');
    if slug.is_empty() {
        format!("mwc-{short_id}")
    } else {
        format!("mwc-{slug}-{short_id}")
    }
}

/// Comments are presentation only; replacing one never rotates the host key.
pub fn public_key_with_comment(public_key: &str, alias: &str) -> Option<String> {
    let mut fields = public_key.split_ascii_whitespace();
    Some(format!("{} {} {alias}", fields.next()?, fields.next()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn readable_aliases_are_single_ssh_tokens() {
        assert_eq!(
            alias("Memeloop Workspace Control Dev", "abcd"),
            "mwc-Memeloop-Workspace-Control-Dev-abcd"
        );
        assert_eq!(alias("运维\nHost *", "abcd"), "mwc-运维-Host-abcd");
        assert_eq!(alias("***", "abcd"), "mwc-abcd");
    }

    #[test]
    fn key_material_is_preserved_when_relabeling_existing_keys() {
        assert_eq!(
            public_key_with_comment("ssh-ed25519 AAAA mwc-workspace-old\n", "mwc-Dev-abcd")
                .as_deref(),
            Some("ssh-ed25519 AAAA mwc-Dev-abcd")
        );
    }
}
