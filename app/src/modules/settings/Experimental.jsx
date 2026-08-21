import { useState } from "react";
import { useTranslation } from "react-i18next";

import { setFeature } from "../../shared/capabilities/features.jsx";
import { Card, GroupTitle, Row, Toggle } from "./parts.jsx";

/**
 * 实验性功能分区。整块由契约驱动:渲染 stage 为 experimental 的全部特性,文案按
 * `settings:features.<id>.title` / `.desc` 约定取——将来加一个特性只需契约一行
 * 加文案两条,这里一行 UI 代码都不必改。
 *
 * 开关本身写在宿主的配置文件里(不是这份 UI 设置),因为拦住能力的那道门在宿主侧,
 * 得在窗口之外也能读到。
 */
export default function Experimental({ features }) {
  const { t } = useTranslation();
  const [busy, setBusy] = useState(null);
  const [error, setError] = useState(null);
  const experimental = features.filter(feature => feature.stage === "experimental");

  const toggle = async (id, next) => {
    setBusy(id);
    setError(null);
    try {
      await setFeature(id, next);
    } catch (e) {
      setError(String(e?.message || e || ""));
    } finally {
      setBusy(null);
    }
  };

  return (
    <div>
      <GroupTitle first right={t("settings:experimental.groupDesc")}>
        {t("settings:experimental.groupTitle")}
      </GroupTitle>
      <Card>
        {experimental.map((feature, index) => (
          <Row key={feature.id} first={index === 0}
            title={t(`settings:features.${feature.id}.title`)}
            desc={t(`settings:features.${feature.id}.desc`)}>
            <span style={{ opacity: busy === feature.id ? 0.45 : 1,
              pointerEvents: busy === feature.id ? "none" : undefined, flex: "none" }}>
              <Toggle on={feature.enabled}
                onChange={next => toggle(feature.id, next)} />
            </span>
          </Row>
        ))}
      </Card>
      {/* 生效时机对所有开关是同一条规则,写在分区上而不是逐个特性抄一遍 */}
      <div style={{ fontSize: 11, color: "var(--tx5)", marginTop: 10, lineHeight: 1.55,
        paddingLeft: 2 }}>
        {t("settings:experimental.togglesNote")}</div>
      {error && <div style={{ fontSize: 11, color: "var(--err-deep)", marginTop: 8,
        paddingLeft: 2, overflowWrap: "anywhere" }} role="alert">{error}</div>}
    </div>
  );
}
