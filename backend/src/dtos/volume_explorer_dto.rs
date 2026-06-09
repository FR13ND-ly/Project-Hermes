use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FileItem {
    pub name: String,
    pub is_dir: bool,
    pub size_bytes: u64,
    pub modified_time: i64,
}
