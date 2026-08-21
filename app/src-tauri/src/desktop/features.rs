//! 特性开关的裁决与读写命令。
//!
//! 宿主是唯一裁决者:事实源是 `~/.ferry/host-settings.json` 的 `features` 命名空间,
//! WebView 只是它的编辑器。真正拦住某个能力的那道门自己回读文件(见 `runtime::mod`
//! 的 `runtime_gate`),不看前端传来的任何东西——门要在窗口起来之前就能判。
//!
//! 哪些特性存在、默认值是什么,全部来自 contracts/features.json 的生成物;这个模块
//! 只负责「读文件 + 校验 id + 落盘」,不认识任何一个具体特性的名字。

use serde::Serialize;

use crate::contracts::features::{default_of, Feature, FEATURES};

use super::host_settings;

/// 设置页要渲染的一行:契约的静态形态 + 这台机器上的当前值。
#[derive(Debug, Serialize)]
pub(crate) struct FeatureState {
    id: &'static str,
    stage: &'static str,
    default: bool,
    enabled: bool,
}

/// 前端传来的 id 不认识时的结构化拒绝。code 稳定可分支,message 兜底展示。
#[derive(Debug, Serialize)]
pub(crate) struct FeatureError {
    code: &'static str,
    feature: String,
    message: String,
}

/// 门直接调用:每次都回文件,不缓存,设置页一改立刻生效,不必重启 App。
pub(crate) fn feature_enabled(feature: Feature) -> bool {
    host_settings::feature_flag(feature.id(), default_of(feature))
}

fn states() -> Vec<FeatureState> {
    // 只列有 `ui` 面的特性:没有界面那一面的特性不该出现在设置页里。
    FEATURES
        .iter()
        .filter(|spec| spec.surfaces.contains(&"ui"))
        .map(|spec| FeatureState {
            id: spec.id,
            stage: spec.stage,
            default: spec.default,
            enabled: feature_enabled(spec.feature),
        })
        .collect()
}

/// 全部特性的当前状态。前端据此渲染设置页,也据此决定各处入口显不显示。
#[tauri::command]
pub(crate) async fn features_list() -> Result<Vec<FeatureState>, String> {
    tauri::async_runtime::spawn_blocking(states)
        .await
        .map_err(|error| error.to_string())
}

/// 改一个特性。只落盘:界面入口是即时的,已经跑起来的 sidecar 跟着 App 的生命周期走。
#[tauri::command]
pub(crate) async fn feature_set(id: String, enabled: bool) -> Result<(), FeatureError> {
    let Some(feature) = Feature::from_id(&id) else {
        return Err(FeatureError {
            code: "feature.unknown",
            message: format!("未知特性开关: {id}"),
            feature: id,
        });
    };
    tauri::async_runtime::spawn_blocking(move || {
        host_settings::set_feature_flag(feature.id(), enabled)
    })
    .await
    .map_err(|error| error.to_string())
    .and_then(|result| result)
    .map_err(|message| FeatureError {
        code: "feature.write_failed",
        feature: feature.id().to_owned(),
        message,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 契约生成物与 id 校验的往返:前端只能用契约里有的 id 进门。
    #[test]
    fn only_ids_declared_by_the_contract_get_through() {
        for spec in FEATURES {
            let feature = Feature::from_id(spec.id).expect("契约里的 id 必须认得");
            assert_eq!(feature, spec.feature);
            assert_eq!(feature.id(), spec.id);
            assert_eq!(default_of(feature), spec.default);
        }
        assert!(Feature::from_id("experimental_agent").is_none());
        assert!(Feature::from_id("").is_none());
        assert!(Feature::from_id("builtin_agent").is_none());
    }

    /// 未知 id 拒绝在落盘之前,而且给的是结构化 code 而不是一句人话。
    #[test]
    fn an_unknown_id_is_refused_before_anything_is_written() {
        let error = tauri::async_runtime::block_on(feature_set("nope".to_owned(), true))
            .expect_err("未知 id 必须被拒");
        let value = serde_json::to_value(&error).expect("可序列化");
        assert_eq!(value["code"], "feature.unknown");
        assert_eq!(value["feature"], "nope");
    }

    /// 设置页拿到的一行必须自带契约默认:前端不重复抄一份默认值。
    #[test]
    fn every_listed_feature_carries_its_contract_shape() {
        for state in states() {
            let spec = FEATURES
                .iter()
                .find(|spec| spec.id == state.id)
                .expect("列出的特性必须来自契约");
            assert_eq!(state.stage, spec.stage);
            assert_eq!(state.default, spec.default);
            assert!(spec.surfaces.contains(&"ui"));
        }
    }
}
