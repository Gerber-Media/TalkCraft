//! File > Import > Audio goes straight to the file dialog and imports what was picked; a file
//! that cannot be read is reported in a dialog, not only in the status line.

use serde_json::json;
use soundcraft_ui_egui::{Services, SoundApp, menus};

fn app_picking(path: String) -> SoundApp {
    let services = Services { pick_open: Some(Box::new(move |_, _| Some(path.clone()))), pick_save: None, pick_folder: None };
    let mut app = SoundApp::new(soundcraft_engine::Engine::default(), None, services);
    app.engine.execute("session.new", &json!({"name": "T", "template": "podcast", "speakers": 1})).unwrap();
    app
}

#[test]
fn import_audio_opens_the_file_dialog_and_imports() {
    let dir = std::env::temp_dir().join(format!("tc-import-picker-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let wav = dir.join("talk.wav");
    let buf = soundcraft_audio_io::AudioBuffer { sample_rate: 48_000, channels: vec![vec![0.1; 4_800]] };
    std::fs::write(&wav, soundcraft_audio_io::encode(&buf, &Default::default()).unwrap()).unwrap();

    let mut app = app_picking(wav.to_string_lossy().into_owned());
    let before = app.engine.session().tracks.len();
    menus::invoke_menu(&mut app, "file.import_audio", "File > Import > Audio...");
    assert!(app.dialogs.open.is_none(), "no path prompt in between");
    assert_eq!(app.engine.session().tracks.len(), before + 1, "imported onto a new track");

    let mut broken = app_picking(dir.to_string_lossy().into_owned());
    menus::invoke_menu(&mut broken, "file.import_audio", "File > Import > Audio...");
    assert!(broken.dialogs.open.is_some(), "the failure is shown");
    let _ = std::fs::remove_dir_all(&dir);
}
