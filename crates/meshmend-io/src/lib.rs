use std::path::PathBuf;

pub fn pick_stl_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("STL mesh", &["stl"])
        .pick_file()
}

pub fn pick_note_session_to_load() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("MeshMend notes", &["json"])
        .pick_file()
}

pub fn pick_note_session_to_save(default_name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("MeshMend notes", &["json"])
        .set_file_name(default_name)
        .save_file()
}
