// 此文件由 scripts/generate-contracts.py 生成，请勿手改。
/// 契约声明的特性开关。id 与变体一一对应：毕业时删掉契约里那一行，这个
/// 变体随之消失，所有引用点变成编译错误，强制清理干净。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Feature {
    BuiltinAgent,
    Handoff,
}

/// 一个特性的静态形态。`surfaces` 声明它有哪几张面，宿主据此决定把它交给谁
/// （比如设置页只列有 `ui` 面的特性）。
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct FeatureSpec {
    pub(crate) feature: Feature,
    pub(crate) id: &'static str,
    pub(crate) stage: &'static str,
    pub(crate) default: bool,
    pub(crate) surfaces: &'static [&'static str],
}

pub(crate) const FEATURES: &[FeatureSpec] = &[
    FeatureSpec {
        feature: Feature::BuiltinAgent,
        id: "builtin-agent",
        stage: "experimental",
        default: false,
        surfaces: &["host-runtime", "ui"],
    },
    FeatureSpec {
        feature: Feature::Handoff,
        id: "handoff",
        stage: "experimental",
        default: false,
        surfaces: &["ui"],
    },
];

impl Feature {
    /// 前端传来的 id 只能经这里进门：不认识的一律返回 None。
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        match id {
            "builtin-agent" => Some(Self::BuiltinAgent),
            "handoff" => Some(Self::Handoff),
            _ => None,
        }
    }

    pub(crate) fn id(self) -> &'static str {
        match self {
            Self::BuiltinAgent => "builtin-agent",
            Self::Handoff => "handoff",
        }
    }
}

/// 契约默认值：配置文件里没写过这个键时用它。
pub(crate) fn default_of(feature: Feature) -> bool {
    match feature {
        Feature::BuiltinAgent => false,
        Feature::Handoff => false,
    }
}
