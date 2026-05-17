use std::path::PathBuf;

pub fn pick_stl_file() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("STL mesh", &["stl"])
        .pick_file()
}

pub fn pick_stl_to_save(default_name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("STL mesh", &["stl"])
        .set_file_name(default_name)
        .save_file()
}

pub fn pick_export_folder() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_folder()
}

pub fn pick_issue_session_to_load() -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("MeshMend inspection issues", &["json"])
        .pick_file()
}

pub fn pick_issue_session_to_save(default_name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("MeshMend inspection issues", &["json"])
        .set_file_name(default_name)
        .save_file()
}

pub fn pick_project_to_save(default_name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("MeshMend project", &["meshmend"])
        .set_file_name(default_name)
        .save_file()
}

pub fn pick_project_to_open() -> Option<PathBuf> {
    rfd::FileDialog::new().pick_folder()
}

pub fn pick_report_to_save(default_name: &str) -> Option<PathBuf> {
    rfd::FileDialog::new()
        .add_filter("Markdown report", &["md"])
        .set_file_name(default_name)
        .save_file()
}
