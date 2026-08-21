//! 宿主自己的小配置:`~/.ferry/host-settings.json`。
//!
//! 目录边界:`~/.ferry` 下的 sqlite(`ferry-state` / `content-index`)归引擎独写,宿主
//! 一个字节都不碰;这份 json 反过来只由宿主读写,引擎不认识它。放在同一个目录只是
//! 因为它属于「这台机器上的 Ferry」,不属于任何一个窗口或工作区。
//!
//! 事实源在文件上而不是 WebView 里:引擎要在窗口起来之前就 spawn,那一刻没有前端
//! 可问。前端的开关只是这份文件的一个编辑器。

use serde_json::Value;
use std::path::{Path, PathBuf};

use super::platform;

/// 「允许 CLI 共享 App 引擎」。设计文档 §7.3:默认开。
///
/// 它是顶层键,不进 `features` 命名空间:那个命名空间装的是有生命周期的特性开关
/// (毕业时整条删掉),而这个是长期存在的稳定配置,两类东西不能混在一起。
const ENGINE_SHARE_KEY: &str = "engine_share";
const DEFAULT_ENGINE_SHARE: bool = true;

/// 特性开关的命名空间:`{"features": {"builtin-agent": true}}`。缺省值不写在这里,
/// 由 contracts/features.json 生成的 `default_of` 给。
const FEATURES_KEY: &str = "features";

fn settings_path() -> Result<PathBuf, String> {
    Ok(platform::home_dir()?
        .join(".ferry")
        .join("host-settings.json"))
}

/// 读一个键:文件缺失、非法 JSON、类型不对,一律回缺省。配置坏掉不能挡住 App 启动。
fn parse_flag(text: &str, key: &str, default: bool) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| value.get(key).and_then(Value::as_bool))
        .unwrap_or(default)
}

/// 只改自己那一个键:同一份文件里还有别的宿主设置,不能整份覆盖掉。
fn render_flag(existing: Option<&str>, key: &str, enabled: bool) -> String {
    let mut object = existing
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default();
    object.insert(key.to_owned(), Value::Bool(enabled));
    format!("{}\n", Value::Object(object))
}

fn read_flag_at(path: &Path, key: &str, default: bool) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .as_deref()
        .map(|text| parse_flag(text, key, default))
        .unwrap_or(default)
}

fn write_flag_at(path: &Path, key: &str, enabled: bool) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("创建 {} 失败: {error}", parent.display()))?;
    }
    let existing = std::fs::read_to_string(path).ok();
    std::fs::write(path, render_flag(existing.as_deref(), key, enabled))
        .map_err(|error| format!("写入 {} 失败: {error}", path.display()))
}

/// 定位不到主目录也要给出缺省,启动不能因此失败。
fn read_flag(key: &str, default: bool) -> bool {
    match settings_path() {
        Ok(path) => read_flag_at(&path, key, default),
        Err(_) => default,
    }
}

fn write_flag(key: &str, enabled: bool) -> Result<(), String> {
    write_flag_at(&settings_path()?, key, enabled)
}

/// 读 `features` 命名空间里的一个键:命名空间缺失、键缺失、类型不对,一律回契约默认。
fn parse_feature(text: &str, id: &str, default: bool) -> bool {
    serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|value| {
            value
                .get(FEATURES_KEY)
                .and_then(|features| features.get(id))
                .and_then(Value::as_bool)
        })
        .unwrap_or(default)
}

/// 只改命名空间里的一个键:同一份文件里还有顶层设置和别的特性,都不能被覆盖掉。
fn render_feature(existing: Option<&str>, id: &str, enabled: bool) -> String {
    let mut object = existing
        .and_then(|text| serde_json::from_str::<Value>(text).ok())
        .and_then(|value| match value {
            Value::Object(map) => Some(map),
            _ => None,
        })
        .unwrap_or_default();
    let mut features = object
        .get(FEATURES_KEY)
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    features.insert(id.to_owned(), Value::Bool(enabled));
    object.insert(FEATURES_KEY.to_owned(), Value::Object(features));
    format!("{}\n", Value::Object(object))
}

fn read_feature_at(path: &Path, id: &str, default: bool) -> bool {
    std::fs::read_to_string(path)
        .ok()
        .as_deref()
        .map(|text| parse_feature(text, id, default))
        .unwrap_or(default)
}

fn write_feature_at(path: &Path, id: &str, enabled: bool) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|error| format!("创建 {} 失败: {error}", parent.display()))?;
    }
    let existing = std::fs::read_to_string(path).ok();
    std::fs::write(path, render_feature(existing.as_deref(), id, enabled))
        .map_err(|error| format!("写入 {} 失败: {error}", path.display()))
}

/// spawn 路径直接调用。
pub(crate) fn engine_share() -> bool {
    read_flag(ENGINE_SHARE_KEY, DEFAULT_ENGINE_SHARE)
}

pub(crate) fn set_engine_share(enabled: bool) -> Result<(), String> {
    write_flag(ENGINE_SHARE_KEY, enabled)
}

/// 特性的门直接调用:每次都回文件,不缓存,改完立刻生效。
pub(crate) fn feature_flag(id: &str, default: bool) -> bool {
    match settings_path() {
        Ok(path) => read_feature_at(&path, id, default),
        Err(_) => default,
    }
}

pub(crate) fn set_feature_flag(id: &str, enabled: bool) -> Result<(), String> {
    write_feature_at(&settings_path()?, id, enabled)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(label: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "ferry-host-settings-{label}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        path
    }

    fn engine_share_of(text: &str) -> bool {
        parse_flag(text, ENGINE_SHARE_KEY, DEFAULT_ENGINE_SHARE)
    }

    /// 唯一一个当前存在的特性,拿它当命名空间读写的样本。
    fn builtin_agent_of(text: &str) -> bool {
        parse_feature(text, "builtin-agent", false)
    }

    #[test]
    fn a_missing_or_broken_file_reads_as_sharing() {
        assert!(engine_share_of("{}"));
        assert!(engine_share_of("not json"));
        assert!(engine_share_of(r#"{"engine_share": "yes"}"#));
        assert!(!engine_share_of(r#"{"engine_share": false}"#));
        assert!(engine_share_of(r#"{"engine_share": true}"#));
    }

    /// 特性开关一律默认关:没写过、命名空间不在、类型不对、坏配置都算「关」。
    #[test]
    fn a_feature_stays_off_until_it_is_written_in() {
        assert!(!builtin_agent_of("{}"));
        assert!(!builtin_agent_of("not json"));
        assert!(!builtin_agent_of(
            r#"{"features": {"builtin-agent": "yes"}}"#
        ));
        assert!(!builtin_agent_of(r#"{"features": {}}"#));
        assert!(!builtin_agent_of(r#"{"engine_share": true}"#));
        // 顶层同名键不算数:特性只认命名空间里的那一份。
        assert!(!builtin_agent_of(r#"{"builtin-agent": true}"#));
        assert!(builtin_agent_of(r#"{"features": {"builtin-agent": true}}"#));
    }

    #[test]
    fn writing_one_key_keeps_the_rest_of_the_file() {
        let rendered = render_flag(Some(r#"{"other": 1}"#), ENGINE_SHARE_KEY, false);
        let value: Value = serde_json::from_str(&rendered).expect("渲染结果是 JSON");
        assert_eq!(value["other"], Value::from(1));
        assert_eq!(value["engine_share"], Value::Bool(false));
        // 特性命名空间与顶层配置共用一份文件,后写的那个不能把先写的抹掉。
        let both = render_feature(Some(&rendered), "builtin-agent", true);
        let value: Value = serde_json::from_str(&both).expect("渲染结果是 JSON");
        assert_eq!(value["engine_share"], Value::Bool(false));
        assert_eq!(value["features"]["builtin-agent"], Value::Bool(true));
        // 同一个命名空间里的兄弟特性也不能被覆盖掉。
        let sibling = render_feature(Some(&both), "another-feature", false);
        let value: Value = serde_json::from_str(&sibling).expect("渲染结果是 JSON");
        assert_eq!(value["features"]["builtin-agent"], Value::Bool(true));
        assert_eq!(value["features"]["another-feature"], Value::Bool(false));
        // 命名空间曾经是别的类型时重建成对象,不把整份内容写丢在错误分支里。
        let salvaged = render_feature(Some(r#"{"features": 7, "other": 1}"#), "x", true);
        let value: Value = serde_json::from_str(&salvaged).expect("渲染结果是 JSON");
        assert_eq!(value["other"], Value::from(1));
        assert_eq!(value["features"]["x"], Value::Bool(true));
        // 文件曾经是数组之类的怪东西时重建成对象,不把整份内容写丢在错误分支里。
        let rebuilt = render_flag(Some("[]"), ENGINE_SHARE_KEY, true);
        assert_eq!(
            serde_json::from_str::<Value>(&rebuilt).expect("渲染结果是 JSON")["engine_share"],
            Value::Bool(true)
        );
    }

    #[test]
    fn the_switch_round_trips_through_a_real_file() {
        let dir = scratch("round-trip");
        let path = dir.join(".ferry").join("host-settings.json");
        // 目录还不存在时读到缺省,写入时自己建目录。
        assert!(read_flag_at(&path, ENGINE_SHARE_KEY, DEFAULT_ENGINE_SHARE));
        write_flag_at(&path, ENGINE_SHARE_KEY, false).expect("可写");
        assert!(!read_flag_at(&path, ENGINE_SHARE_KEY, DEFAULT_ENGINE_SHARE));
        write_flag_at(&path, ENGINE_SHARE_KEY, true).expect("可改回");
        assert!(read_flag_at(&path, ENGINE_SHARE_KEY, DEFAULT_ENGINE_SHARE));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_feature_round_trips_without_touching_the_top_level_settings() {
        let dir = scratch("feature-round-trip");
        let path = dir.join(".ferry").join("host-settings.json");
        assert!(!read_feature_at(&path, "builtin-agent", false));
        write_flag_at(&path, ENGINE_SHARE_KEY, false).expect("可写");
        write_feature_at(&path, "builtin-agent", true).expect("可写");
        assert!(read_feature_at(&path, "builtin-agent", false));
        assert!(!read_flag_at(&path, ENGINE_SHARE_KEY, DEFAULT_ENGINE_SHARE));
        write_feature_at(&path, "builtin-agent", false).expect("可改回");
        assert!(!read_feature_at(&path, "builtin-agent", false));
        // 没写过的特性照旧回落到调用方给的契约默认,不受兄弟特性影响。
        assert!(read_feature_at(&path, "never-written", true));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
