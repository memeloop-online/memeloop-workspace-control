use std::collections::{BTreeMap, BTreeSet};

use super::{InjectionError, InjectionItem, InjectionKind, InjectionScope, ResolvedInjection};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum InjectionDestinationKind {
    Environment,
    File,
}

pub fn resolve_injections(
    organization: &[InjectionItem],
    user: &[InjectionItem],
    workspace: &[InjectionItem],
) -> Result<Vec<ResolvedInjection>, InjectionError> {
    let mut resolved = BTreeMap::<(InjectionDestinationKind, String), ResolvedInjection>::new();

    apply_scope(&mut resolved, InjectionScope::Organization, organization)?;
    apply_scope(&mut resolved, InjectionScope::User, user)?;
    apply_scope(&mut resolved, InjectionScope::Workspace, workspace)?;

    Ok(resolved.into_values().collect())
}

fn apply_scope(
    resolved: &mut BTreeMap<(InjectionDestinationKind, String), ResolvedInjection>,
    scope: InjectionScope,
    items: &[InjectionItem],
) -> Result<(), InjectionError> {
    let mut seen_keys = BTreeSet::<&str>::new();
    let mut seen_destinations = BTreeSet::<(InjectionDestinationKind, &str)>::new();
    for item in items {
        if item.key.is_empty() {
            return Err(InjectionError::EmptyKey);
        }
        if item.locked && scope != InjectionScope::Organization {
            return Err(InjectionError::LockOutsideOrganization {
                key: item.key.clone(),
                scope,
            });
        }
        if !seen_keys.insert(&item.key) {
            return Err(InjectionError::DuplicateInScope {
                key: item.key.clone(),
                scope,
            });
        }
        let destination_kind = destination_kind(item.kind);
        let destination = (destination_kind, item.target.as_str());
        if !seen_destinations.insert(destination) {
            return Err(InjectionError::DuplicateDestinationInScope {
                kind: item.kind,
                target: item.target.clone(),
                scope,
            });
        }
        let destination = (destination_kind, item.target.clone());
        if let Some(existing) = resolved.get(&destination)
            && existing.item.locked
        {
            return Err(InjectionError::LockedOverride {
                key: existing.item.key.clone(),
                kind: item.kind,
                target: item.target.clone(),
                attempted_scope: scope,
            });
        }
        resolved.insert(
            destination,
            ResolvedInjection {
                source: scope,
                item: item.clone(),
            },
        );
    }
    Ok(())
}

fn destination_kind(kind: InjectionKind) -> InjectionDestinationKind {
    match kind {
        InjectionKind::EnvironmentVariable => InjectionDestinationKind::Environment,
        InjectionKind::SecretFile | InjectionKind::ConfigFile | InjectionKind::SshPublicKey => {
            InjectionDestinationKind::File
        }
    }
}
