use thiserror::Error;

use super::{InjectionKind, InjectionScope};

#[derive(Debug, Error, PartialEq, Eq)]
pub enum InjectionError {
    #[error("injection key must not be empty")]
    EmptyKey,
    #[error(
        "injection key must use 1-128 letters, digits, spaces, dots, underscores, or hyphens without leading or trailing spaces"
    )]
    InvalidKey,
    #[error("injection target is invalid for its kind")]
    InvalidTarget,
    #[error("file mode must be between 000 and 777")]
    InvalidFileMode,
    #[error("owner and group must be safe local account names")]
    InvalidOwnerOrGroup,
    #[error("template selector must be a template UUID or *")]
    InvalidTemplateSelector,
    #[error("label selectors must use non-empty bounded keys and string values")]
    InvalidLabelSelector,
    #[error("Base64 injection value is invalid")]
    InvalidBase64,
    #[error("environment variable values must be UTF-8 without control characters")]
    InvalidEnvironmentValue,
    #[error("SSH public key injection must contain one valid OpenSSH public key")]
    InvalidSshPublicKey,
    #[error("duplicate injection {key} in {scope:?} scope")]
    DuplicateInScope { key: String, scope: InjectionScope },
    #[error("duplicate {kind:?} injection target {target} in {scope:?} scope")]
    DuplicateDestinationInScope {
        kind: InjectionKind,
        target: String,
        scope: InjectionScope,
    },
    #[error("only organization injections may be locked: {key} in {scope:?} scope")]
    LockOutsideOrganization { key: String, scope: InjectionScope },
    #[error(
        "{attempted_scope:?} injection cannot override locked organization injection {key} at {kind:?} target {target}"
    )]
    LockedOverride {
        key: String,
        kind: InjectionKind,
        target: String,
        attempted_scope: InjectionScope,
    },
}
