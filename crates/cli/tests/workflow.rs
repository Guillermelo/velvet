use std::process::Command;
fn run(root: &std::path::Path, args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_velvet"))
        .arg("--project")
        .arg(root)
        .args(args)
        .output()
        .unwrap()
}
#[test]
fn cli_persists_commands_history_and_renders_without_ai() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("song");
    assert!(run(&root, &["new", root.to_str().unwrap()])
        .status
        .success());
    assert!(run(&root, &["track", "add", "--name", "Vocals"])
        .status
        .success());
    let project = velvet_core::load(&root).unwrap();
    let tid = &project.tracks[0].id;
    let source = temp.path().join("external.wav");
    velvet_audio::export(
        &velvet_audio::Mix {
            sample_rate: 8000,
            frames: vec![[0.2; 2]; 8000],
            missing: vec![],
            sources: vec![],
            peak: 0.2,
        },
        &source,
    )
    .unwrap();
    let bytes = std::fs::read(&source).unwrap();
    assert!(run(
        &root,
        &["clip", "import", "--track", tid, source.to_str().unwrap()]
    )
    .status
    .success());
    assert!(run(&root, &["track", "volume", tid, "-3"]).status.success());
    assert_eq!(
        velvet_core::load(&root).unwrap().tracks[0].mixer.volume_db,
        -3.0
    );
    assert!(run(&root, &["undo"]).status.success());
    assert_eq!(
        velvet_core::load(&root).unwrap().tracks[0].mixer.volume_db,
        0.0
    );
    assert!(run(&root, &["redo"]).status.success());
    assert_eq!(
        velvet_core::load(&root).unwrap().tracks[0].mixer.volume_db,
        -3.0
    );
    assert!(!run(&root, &["track", "pan", tid, "2"]).status.success());
    assert!(run(
        &root,
        &["render", root.join("renders/mix.wav").to_str().unwrap()]
    )
    .status
    .success());
    assert_eq!(
        velvet_audio::decode(&root.join("renders/mix.wav"))
            .unwrap()
            .frames
            .len(),
        48000
    );
    assert_eq!(std::fs::read(source).unwrap(), bytes);
    assert!(!root.join("media").exists());
    // A manual YAML edit invalidates stale CLI undo history.
    let mut project = velvet_core::load(&root).unwrap();
    project.tracks[0].name = "Manual edit".into();
    velvet_core::save(&root, &project).unwrap();
    assert!(!run(&root, &["undo"]).status.success());
    assert_eq!(
        velvet_core::load(&root).unwrap().tracks[0].name,
        "Manual edit"
    );
}
