use super::*;

#[test]
fn workspace_id_query_is_deduplicated_and_bounded() {
    let first = Uuid::parse_str("01a05874-0f29-78f2-95ca-086b4debca09").unwrap();
    let second = Uuid::parse_str("01a05875-87b8-74c1-a252-412a71050991").unwrap();
    let parsed = parse_workspace_ids(&format!("{second},{first},{second}")).unwrap();
    assert_eq!(parsed, vec![first, second]);

    assert!(parse_workspace_ids("").is_err());
    assert!(parse_workspace_ids("not-a-uuid").is_err());
    let too_many = (0..101)
        .map(|_| Uuid::now_v7().to_string())
        .collect::<Vec<_>>()
        .join(",");
    assert!(parse_workspace_ids(&too_many).is_err());
}
