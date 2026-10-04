use std::{fs, io::Write as _, path::PathBuf};

use pyrrhic_rs::{Color, EngineAdapter, TableBases};

#[derive(Clone)]
struct NoProbe;

// This integration test checks public discovery metadata without probing a
// chess position; a move-choice TSV cannot express its contract.
impl EngineAdapter for NoProbe {
    fn pawn_attacks(_: Color, _: u64) -> u64 {
        unreachable!()
    }
    fn knight_attacks(_: u64) -> u64 {
        unreachable!()
    }
    fn bishop_attacks(_: u64, _: u64) -> u64 {
        unreachable!()
    }
    fn rook_attacks(_: u64, _: u64) -> u64 {
        unreachable!()
    }
    fn queen_attacks(_: u64, _: u64) -> u64 {
        unreachable!()
    }
    fn king_attacks(_: u64) -> u64 {
        unreachable!()
    }
}

struct TestDir(PathBuf);

impl TestDir {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!("pyrrhic-discovery-{}", std::process::id()));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn add(&self, name: &str) {
        fs::File::create(self.0.join(name))
            .unwrap()
            .write_all(&[0; 80])
            .unwrap();
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        fs::remove_dir_all(&self.0).unwrap();
    }
}

#[test]
fn dtm_only_files_do_not_extend_wdl_capability() {
    let directory = TestDir::new();
    directory.add("KQvK.rtbw");
    directory.add("KQQvKQ.rtbm");

    let tables = TableBases::<NoProbe>::new(directory.0.to_str().unwrap()).unwrap();
    assert_eq!(tables.max_pieces(), 3);
    assert_eq!(tables.materials(), vec![("KQvK".to_owned(), false)]);
}
