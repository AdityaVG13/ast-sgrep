use super::*;
use tempfile::TempDir;

#[test]
fn bump_advances_and_peers_observe() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    assert_eq!(read_writer_generation(root, None), 0);
    let g1 = bump_writer_generation(root, None).unwrap();
    assert_ne!(g1, 0);
    assert_eq!(read_writer_generation(root, None), g1);
    let g2 = bump_writer_generation(root, None).unwrap();
    assert_ne!(g2, g1);
    let path = writer_generation_path(root, None);
    assert!(path.starts_with(root.join(INDEX_DIR)));
    assert_eq!(
        std::fs::read_to_string(&path).unwrap().trim(),
        g2.to_string()
    );
}

#[test]
fn concurrent_bumps_never_publish_the_same_epoch() {
    use std::collections::HashSet;
    use std::sync::Mutex;
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let published = Mutex::new(Vec::new());
    std::thread::scope(|scope| {
        for _ in 0..8 {
            scope.spawn(|| {
                let epoch = bump_writer_generation(root, None).unwrap();
                published.lock().unwrap().push(epoch);
            });
        }
    });
    let values = published.into_inner().unwrap();
    let unique: HashSet<u64> = values.iter().copied().collect();
    assert_eq!(
        unique.len(),
        values.len(),
        "duplicate writer epochs: {values:?}"
    );
    let on_disk = read_writer_generation(root, None);
    assert!(
        unique.contains(&on_disk),
        "file epoch {on_disk} missing from published {values:?}"
    );
}

#[test]
fn pinned_db_stamp_lives_beside_db() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let db = root.join("custom").join("index.db");
    std::fs::create_dir_all(db.parent().unwrap()).unwrap();
    let g = bump_writer_generation(root, Some(&db)).unwrap();
    assert_ne!(g, 0);
    assert_eq!(read_writer_generation(root, Some(&db)), g);
    assert_eq!(
        writer_generation_path(root, Some(&db)),
        root.join("custom").join(WRITER_GENERATION_FILE)
    );
}

#[test]
fn generation_candidate_db_stamps_index_home() {
    let temp = TempDir::new().unwrap();
    let root = temp.path();
    let candidate = root
        .join(INDEX_DIR)
        .join(GENERATIONS_DIR)
        .join("000001")
        .join("index.db");
    let g = bump_writer_generation(root, Some(&candidate)).unwrap();
    assert_ne!(g, 0);
    assert_eq!(read_writer_generation(root, Some(&candidate)), g);
    assert_eq!(
        writer_generation_path(root, Some(&candidate)),
        root.join(INDEX_DIR).join(WRITER_GENERATION_FILE)
    );
}
