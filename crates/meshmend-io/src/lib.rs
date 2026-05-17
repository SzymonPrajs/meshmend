use std::path::PathBuf;

pub fn pick_stl_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("STL mesh", &["stl"])
        .pick_file()
}
