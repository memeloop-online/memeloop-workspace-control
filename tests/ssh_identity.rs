use memeloop_workspace_control::{crypto::EnvelopeCipher, storage::Database};
use uuid::Uuid;

#[tokio::test]
async fn workspace_host_key_is_stable_encrypted_and_openssh_compatible() {
    let database = Database::connect("sqlite::memory:", "ssh-identity".parse().unwrap())
        .await
        .unwrap();
    database.migrate().await.unwrap();
    let cipher =
        EnvelopeCipher::from_base64("AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
    let workspace_id = Uuid::now_v7();
    let first = database
        .ensure_workspace_ssh_identity(&cipher, workspace_id, 1)
        .await
        .unwrap();
    let second = database
        .ensure_workspace_ssh_identity(&cipher, workspace_id, 2)
        .await
        .unwrap();
    assert_eq!(first.public.public_key, second.public.public_key);
    assert_eq!(first.public.fingerprint, second.public.fingerprint);
    assert_eq!(first.private_key.as_str(), second.private_key.as_str());
    let parsed = ssh_key::PrivateKey::from_openssh(first.private_key.as_bytes()).unwrap();
    assert_eq!(
        parsed.public_key().to_openssh().unwrap(),
        first.public.public_key
    );
    assert_eq!(
        parsed.fingerprint(ssh_key::HashAlg::Sha256).to_string(),
        first.public.fingerprint
    );
    assert!(!format!("{first:?}").contains("BEGIN OPENSSH PRIVATE KEY"));

    let public = database
        .workspace_ssh_public_identity(workspace_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(public.fingerprint, first.public.fingerprint);
    let snapshot = serde_json::to_string(&database.export_snapshot(3).await.unwrap()).unwrap();
    assert!(!snapshot.contains("BEGIN OPENSSH PRIVATE KEY"));
    assert!(snapshot.contains(&first.public.fingerprint));
}
