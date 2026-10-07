use eplyx_server::{project::ProjectToken, registry::Registry, storage::Storage};
use std::sync::{Arc, Barrier};

#[test]
fn recording_an_in_flight_request_never_reactivates_a_revoked_token() {
    let scratch = tempfile::tempdir().unwrap();
    let registry = Registry::new(Storage::open(scratch.path()).unwrap());
    let token = ProjectToken::new("tok_test", "proj_test", "CI", "test-secret").unwrap();
    registry.create_token(&token).unwrap();
    let authenticated = registry
        .authenticate_token("proj_test", "test-secret")
        .unwrap();

    registry.revoke_token("proj_test", "tok_test").unwrap();
    // Authentication and request accounting are separate operations. A request
    // already in flight can hold the old record when an owner revokes it.
    registry.note_token_use(&authenticated);

    assert!(registry
        .authenticate_token("proj_test", "test-secret")
        .is_err());
    assert!(registry.list_tokens("proj_test").unwrap()[0].is_revoked());
}

#[test]
fn concurrent_writes_preserve_complete_documents_and_distinct_extensions() {
    let scratch = tempfile::tempdir().unwrap();
    let storage = Storage::open(scratch.path()).unwrap();
    let barrier = Arc::new(Barrier::new(8));
    let threads: Vec<_> = (0..8)
        .map(|index| {
            let storage = storage.clone();
            let barrier = barrier.clone();
            let path = scratch.path().join(format!("report.{index}"));
            std::thread::spawn(move || {
                let document = vec![index as u8; 256 * 1024];
                barrier.wait();
                storage.write_bytes(&path, &document).unwrap();
                assert_eq!(std::fs::read(path).unwrap(), document);
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }

    let shared = scratch.path().join("shared.json");
    let threads: Vec<_> = (0..8)
        .map(|index| {
            let storage = storage.clone();
            let barrier = barrier.clone();
            let shared = shared.clone();
            std::thread::spawn(move || {
                barrier.wait();
                storage
                    .write_bytes(&shared, &vec![index as u8; 256 * 1024])
                    .unwrap();
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let stored = std::fs::read(shared).unwrap();
    assert_eq!(stored.len(), 256 * 1024);
    assert!(stored.iter().all(|byte| *byte == stored[0]));
}
