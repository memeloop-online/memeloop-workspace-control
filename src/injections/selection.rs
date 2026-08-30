use std::collections::BTreeSet;

use uuid::Uuid;

use super::InjectionItem;
use crate::workspaces::AccessMode;

#[derive(Debug, Clone, Copy)]
pub struct InjectionSelection<'a> {
    pub workspace_id: Option<Uuid>,
    pub organization_id: Uuid,
    pub owner_id: Uuid,
    pub template_id: Option<Uuid>,
    pub image: &'a str,
    pub access_mode: AccessMode,
}

pub fn select_injections(
    items: &[InjectionItem],
    selection: InjectionSelection<'_>,
) -> Vec<InjectionItem> {
    items
        .iter()
        .filter(|item| injection_matches(item, selection))
        .cloned()
        .collect()
}

pub fn filter_injection_refs(
    items: &[InjectionItem],
    refs: Option<&[String]>,
    include_locked: bool,
) -> Vec<InjectionItem> {
    let Some(refs) = refs else {
        return items.to_vec();
    };
    let selected = refs.iter().map(String::as_str).collect::<BTreeSet<_>>();
    items
        .iter()
        .filter(|item| (include_locked && item.locked) || selected.contains(item.key.as_str()))
        .cloned()
        .collect()
}

fn injection_matches(item: &InjectionItem, selection: InjectionSelection<'_>) -> bool {
    let template_matches = item.template_selector.as_deref().is_none_or(|selector| {
        selector == "*"
            || selection
                .template_id
                .is_some_and(|template_id| selector == template_id.to_string())
    });
    template_matches
        && item.labels.iter().all(|(key, expected)| {
            selection_label(selection, key).is_some_and(|actual| actual == *expected)
        })
}

fn selection_label(selection: InjectionSelection<'_>, key: &str) -> Option<String> {
    match key {
        "workspace_id" => selection.workspace_id.map(|value| value.to_string()),
        "organization_id" => Some(selection.organization_id.to_string()),
        "owner_id" => Some(selection.owner_id.to_string()),
        "template_id" => selection.template_id.map(|value| value.to_string()),
        "image" => Some(selection.image.to_owned()),
        "access_mode" => Some(selection.access_mode.as_str().to_owned()),
        _ => None,
    }
}
