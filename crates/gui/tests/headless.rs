//! Actual Rune shell metadata, without opening a desktop window.
use std::process::Command;

#[test]
fn headless_tree_reports_the_accepted_source_in_the_native_title() {
    let dir = tempfile::tempdir().unwrap();
    for (scene, title) in
        [("empty", "ASCII Painter"), ("image", "ASCII Painter — sunset.png"), ("video", "ASCII Painter — sunset.mp4")]
    {
        let tree = dir.path().join(format!("{scene}.json"));
        let result = Command::new(env!("CARGO_BIN_EXE_ascii-painter"))
            .args(["--scene", scene, "--tree"])
            .arg(&tree)
            .args(["--at", "0.5", "--scale", "1"])
            .output()
            .unwrap();
        assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));
        let tree: serde_json::Value = serde_json::from_slice(&std::fs::read(tree).unwrap()).unwrap();
        assert_eq!(tree["title"], title, "scene={scene}");
    }
}
