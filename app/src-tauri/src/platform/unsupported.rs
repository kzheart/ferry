use std::path::Path;

pub(super) fn reveal_path(_path: &Path) -> Result<(), String> {
    Err("当前平台尚未支持在文件管理器中定位".to_owned())
}
