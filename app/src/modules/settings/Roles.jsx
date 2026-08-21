// 设置 · 角色:左侧角色列表 + 右侧按语义分组的详情(身份 / 人设 / 能力 / 模型 / 安全)。
// 内置角色同样可改,改动存成覆盖层,随时能恢复出厂设置;角色配置可整体或单个导出为 json 文件再导入。
import { useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import { exportRolesFile, importRolesFile } from "../../platform/desktop/client.js";
import { useFerryRuntime } from "../../shared/capabilities/ferryRuntime.jsx";
import RoleSkillPicker from "./RoleSkillPicker.jsx";
import { CopyIcon, RoleAvatar } from "../../shared/ui/icons.jsx";
import { Card, GroupTitle, Row, Select, Toggle, inputStyle } from "./parts.jsx";
import RoleIconPicker from "./RoleIconPicker.jsx";
import RoleList from "./RoleList.jsx";
import RoleToolGrid from "./RoleToolGrid.jsx";
import { TOOLS, blankRole, editable, modelKey, payload } from "./roleForm.js";
import { EXPORT_GLYPH, GROUP_GLYPH, INFO_GLYPH, UNDO_GLYPH, glyph }
  from "./roleGlyphs.jsx";
import {
  buildRoleBundle, parseRoleBundle, planRoleImport, roleBundleFileName,
} from "./roleBundle.js";

// optimizerFeature:「会话优化」测试中功能的开关;关闭时不提供优化器标记入口
export default function Roles({ optimizerFeature = false }) {
  const { t } = useTranslation();
  const ferry = useFerryRuntime();
  const [selectedId, setSelectedId] = useState("default");
  const [draft, setDraft] = useState(null);
  const [creating, setCreating] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [loading, setLoading] = useState(true);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [transferOpen, setTransferOpen] = useState(false);
  const [confirming, setConfirming] = useState(false);
  const detailRef = useRef(null);
  const avatarRef = useRef(null);

  const selected = useMemo(
    () => ferry.roles.find(role => role.id === selectedId) || ferry.roles[0],
    [ferry.roles, selectedId]);
  // 内置角色可编辑,只是不能删除、不能改 ID,并且多一个恢复出厂设置的出口
  const builtinSelected = !creating && Boolean(selected?.builtin);
  const baseline = useMemo(
    () => (creating ? blankRole() : selected ? editable(selected) : null), [creating, selected]);
  // 切角色时在渲染期就把草稿换掉,不能等 effect:effect 跑在提交之后,
  // 中间那一帧 draft 还是上一个角色的,dirty 会为真,标题栏闪出「放弃更改 / 保存更改」。
  // 用 baseline 的内容做键,恢复出厂设置那种"选中项没变但内容变了"的情况也能跟上。
  const [syncedKey, setSyncedKey] = useState(null);
  const baselineKey = creating ? "@new" : JSON.stringify(baseline);
  if (baselineKey !== syncedKey) {
    setSyncedKey(baselineKey);
    setDraft(baseline);
  }
  const dirty = Boolean(draft && baseline && JSON.stringify(draft) !== JSON.stringify(baseline));

  const reload = async () => {
    setLoading(true);
    setError("");
    try {
      await ferry.reloadRoles();
      // 技能选择器只列已导入的技能,进这一页就得先把库拉回来
      await ferry.reloadSkills();
    } catch (error2) {
      setError(error2.message || String(error2));
    } finally {
      setLoading(false);
    }
  };
  useEffect(() => { void reload(); }, []);
  // 换角色时收起所有临时态,否则删除确认会跟着停在下一个角色上
  useEffect(() => {
    setConfirming(false);
    setPickerOpen(false);
    detailRef.current?.scrollTo({ top: 0 });
  }, [selectedId, creating]);

  const mutate = async action => {
    setBusy(true);
    setError("");
    try {
      await action();
    } catch (error2) {
      setError(error2.message || String(error2));
    } finally {
      setBusy(false);
    }
  };

  const patch = value => setDraft(current => ({ ...current, ...value }));

  const save = () => mutate(async () => {
    const role = payload(draft);
    if (creating) await ferry.createRole(role);
    else await ferry.updateRole(role);
    setSelectedId(role.id);
    setCreating(false);
    setNotice("");
  });

  const startCreate = () => {
    setCreating(true);
    setError("");
    setNotice("");
  };

  const copy = () => mutate(async () => {
    const source = creating ? null : selected;
    if (!source) return;
    const result = await ferry.copyRole(source.id,
      `${source.id}-copy-${Date.now().toString(36)}`,
      `${source.name} ${t("settings:roles.copySuffix")}`);
    setSelectedId(result.id);
    setCreating(false);
  });

  const remove = () => mutate(async () => {
    await ferry.deleteRole(selected.id);
    setSelectedId("default");
  });

  const restore = () => mutate(async () => {
    await ferry.resetRole(selected.id);
    setConfirming(false);
    setNotice(t("settings:roles.resetDone"));
  });

  const exportRoles = roles => mutate(async () => {
    setNotice("");
    const path = await exportRolesFile(roleBundleFileName(roles),
      `${JSON.stringify(buildRoleBundle(roles), null, 2)}\n`);
    if (path) setNotice(t("settings:roles.exportDone", { path }));
  });

  const importRoles = () => mutate(async () => {
    setNotice("");
    const text = await importRolesFile();
    if (text === null) return;
    let incoming;
    try {
      incoming = parseRoleBundle(text);
    } catch (reason) {
      const key = `import${reason.message.charAt(0).toUpperCase()}${reason.message.slice(1)}`;
      throw new Error(t(`settings:roles.${key}`, t("settings:roles.importNotBundle")));
    }
    const plan = planRoleImport(incoming, ferry.roles.map(role => role.id));
    const failures = [];
    let created = 0;
    let lastId = null;
    for (const item of plan) {
      try {
        // 逐个提交:一个角色字段非法不该连累整份文件
        await ferry.createRole(payload({ ...blankRole(), ...item.role }));
        created += 1;
        lastId = item.role.id;
      } catch (reason) {
        failures.push(reason.message || String(reason));
      }
    }
    const renamed = plan.filter(item => item.renamedFrom).length;
    const lines = [t("settings:roles.importDone", { n: created })];
    if (renamed) lines.push(t("settings:roles.importRenamed", { n: renamed }));
    setNotice(lines.join(" · "));
    if (failures.length) {
      setError(t("settings:roles.importPartial",
        { n: failures.length, reason: failures[0] }));
    }
    if (lastId) { setCreating(false); setSelectedId(lastId); }
  });

  if (!draft) {
    return (
      <div style={{ flex: 1, display: "grid", placeItems: "center", padding: 32 }}>
        <div style={{ maxWidth: 340, textAlign: "center" }}>
          <div style={{ fontSize: 13, fontWeight: 600, color: "var(--tx2)" }}>
            {loading ? t("settings:roles.loading") : t("settings:roles.unavailable")}</div>
          <div style={{ marginTop: 6, fontSize: 11.5, lineHeight: 1.6, color: "var(--tx5)" }}>
            {loading ? t("settings:roles.loadingDesc")
              : error || t("settings:roles.unavailableDesc")}</div>
          {!loading && (
            <button className="fbtn" style={{ marginTop: 14 }} onClick={reload}>
              {t("settings:roles.retry")}</button>)}
        </div>
      </div>
    );
  }

  const enabled = draft.tools.length;
  const currentModel = modelKey(draft.model);
  const matched = (ferry.models || []).find(
    model => `${model.provider}/${model.id}` === currentModel);
  const knownModel = Boolean(matched);
  const modelLabel = matched ? (matched.name || matched.id) : currentModel;
  const control = { ...inputStyle, width: 320, maxWidth: "100%" };
  // 标题栏的次级动作:只留图标,文字留给 title,免得三个按钮把头部压满
  const iconAction = (title, onClick, icon) => (
    <button type="button" className="hov" title={title} aria-label={title}
      disabled={busy} onClick={onClick}
      style={{ width: 30, height: 30, border: "none", borderRadius: 8, flex: "none",
        background: "transparent", color: "var(--tx3b)", cursor: "default",
        display: "inline-flex", alignItems: "center", justifyContent: "center" }}>
      {icon}</button>
  );
  return (
    <div style={{ flex: 1, minHeight: 0, display: "flex" }}>
      <RoleList roles={ferry.roles} selectedId={selected?.id} creating={creating}
        draft={draft} busy={busy} transferOpen={transferOpen}
        deletable={!creating && !!selected && !selected.builtin}
        deleteName={selected?.name} onDelete={remove}
        onSelect={id => { setSelectedId(id); setCreating(false); }}
        onCreate={startCreate} onToggleTransfer={setTransferOpen}
        transfer={{
          onExportAll: () => exportRoles(ferry.roles),
          onImport: importRoles,
        }} />

      {/* 详情:动作跟着标题走,底部不再有工具条,右侧也就没有那条与左栏对不齐的分隔线 */}
      <div ref={detailRef} className="fscroll"
        style={{ flex: 1, minWidth: 0, overflowY: "auto", padding: "0 24px 24px" }}>
        <div style={{ maxWidth: 640, margin: "0 auto" }}>
          {/* 身份头部:粘在顶部,表单再长也不用滚回去保存 */}
          <div style={{ position: "sticky", top: 0, zIndex: 5, background: "var(--settings-bg)",
            display: "flex", alignItems: "center", gap: 13, padding: "18px 0 16px",
            borderBottom: "1px solid var(--line4)" }}>
              <div style={{ flex: "none" }}>
                <button type="button" ref={avatarRef}
                  onClick={() => setPickerOpen(value => !value)}
                  title={t("settings:roles.iconPickerTitle")}
                  style={{ border: "none", background: "transparent", padding: 0,
                    cursor: "default", display: "block", borderRadius: 14 }}>
                  <RoleAvatar icon={draft.icon} color={draft.color} size={44} />
                </button>
                {pickerOpen && (
                  <RoleIconPicker anchorRef={avatarRef} value={draft.icon} color={draft.color}
                    onPick={patch} onClose={() => setPickerOpen(false)} />)}
              </div>
              <div style={{ minWidth: 0, flex: 1 }}>
                <div style={{ fontSize: 17, fontWeight: 600, color: "var(--tx1)",
                  display: "flex", alignItems: "center", gap: 8, letterSpacing: "-.01em" }}>
                  {draft.name || t("settings:roles.create")}
                  {builtinSelected && (
                    <span style={{ fontSize: 10.5, fontWeight: 600, padding: "2px 7px",
                      borderRadius: 5, background: "var(--acc-soft3)", color: "var(--acc-text)",
                      border: "1px solid var(--acc-line)" }}>
                      {t("settings:roles.builtin")}</span>)}
                </div>
                <div style={{ fontSize: 11.5, color: "var(--tx5)", marginTop: 4 }}>
                  {[draft.description.trim(),
                    t("settings:roles.toolCount", { n: enabled }),
                    modelLabel || t("settings:roles.metaFollowModel"),
                  ].filter(Boolean).join(" · ")}</div>
              </div>
              <div style={{ flex: "none", display: "flex", alignItems: "center", gap: 4 }}>
                {!creating && selected && iconAction(
                  t("settings:roles.copy"), copy, <CopyIcon size={14} />)}
                {!creating && selected && iconAction(
                  t("settings:roles.exportOne"), () => exportRoles([selected]),
                  glyph(EXPORT_GLYPH, 14))}
                {(creating || dirty) && iconAction(
                  t(creating ? "settings:roles.cancel" : "settings:roles.discard"),
                  () => { setCreating(false); setDraft(editable(selected)); },
                  glyph(UNDO_GLYPH, 14))}
                {/* 尺寸对齐左边那排图标按钮:不给高度会落到浏览器默认内边距,
                    在 30px 的图标中间缩成一枚黑色小片,看着像贴上去的 */}
                {(creating || dirty) && (
                  <button className="fbtn-primary" onClick={save}
                    style={{ marginLeft: 4, height: 30, padding: "0 13px",
                      borderRadius: 8 }}
                    disabled={busy || !draft.name.trim() || !draft.id.trim()}>
                    {t(creating ? "settings:roles.createSave" : "settings:roles.save")}</button>)}
              </div>
            </div>

            {notice && (
              <div style={{ marginTop: 14, fontSize: 11.5, color: "var(--ok-body2)",
                background: "var(--ok-bg)", border: "1px solid var(--ok-line)",
                borderRadius: 8, padding: "8px 11px", overflowWrap: "anywhere" }}>
                {notice}</div>)}
            {error && (
              <div style={{ marginTop: 14, fontSize: 11.5, color: "var(--err-text)",
                background: "var(--err-bg)", border: "1px solid var(--err-line)",
                borderRadius: 8, padding: "8px 11px", overflowWrap: "anywhere" }}>
                {error}</div>)}

            <GroupTitle icon={glyph(GROUP_GLYPH.identity)}>
              {t("settings:roles.groupIdentity")}</GroupTitle>
            <Card>
              <Row first title={t("settings:roles.name")}>
                <input value={draft.name} style={control}
                  onChange={event => patch({ name: event.target.value })} />
              </Row>
              <Row title={t("settings:roles.description")}
                desc={t("settings:roles.descriptionHint")}>
                <input value={draft.description} style={control}
                  onChange={event => patch({ description: event.target.value })} />
              </Row>
              <Row title={t("settings:roles.id")} desc={t("settings:roles.idHint")}>
                {creating ? (
                  <input value={draft.id} className="mono" style={control}
                    placeholder="code-reviewer"
                    onChange={event => patch({ id: event.target.value })} />
                ) : (
                  <span style={{ display: "inline-flex", alignItems: "center", gap: 7,
                    fontSize: 12, color: "var(--tx3b)" }}>
                    <span className="mono selectable">{draft.id}</span>
                    <button className="hov" title={t("settings:roles.idCopy")}
                      onClick={() => navigator.clipboard?.writeText(draft.id)}
                      style={{ width: 24, height: 24, border: "none", borderRadius: 6,
                        background: "transparent", color: "var(--tx4)", cursor: "default",
                        display: "inline-flex", alignItems: "center", justifyContent: "center" }}>
                      <CopyIcon size={12} />
                    </button>
                  </span>
                )}
              </Row>
            </Card>

            <GroupTitle icon={glyph(GROUP_GLYPH.persona)}>
              {t("settings:roles.groupPersona")}</GroupTitle>
            <Card>
              <div style={{ padding: "12px 14px 10px" }}>
                <textarea value={draft.persona} rows={6}
                  placeholder={t("settings:roles.personaPlaceholder")}
                  onChange={event => patch({ persona: event.target.value })}
                  className="mono"
                  style={{ width: "100%", minHeight: 132, border: "none", outline: "none",
                    resize: "vertical", background: "transparent", color: "var(--tx1)",
                    fontSize: 12, lineHeight: 1.72 }} />
                <div style={{ display: "flex", gap: 10, marginTop: 8, paddingTop: 9,
                  borderTop: "1px solid var(--line6)", fontSize: 10.5, color: "var(--tx5)" }}>
                  <span>{t("settings:roles.personaHint")}</span>
                  <span className="mono" style={{ marginLeft: "auto" }}>
                    {t("settings:roles.personaCount", { n: draft.persona.length })}</span>
                </div>
              </div>
            </Card>

            <GroupTitle icon={glyph(GROUP_GLYPH.capability)}
              right={t("settings:roles.capabilityCount", { n: enabled, total: TOOLS.length })}>
              {t("settings:roles.groupCapability")}</GroupTitle>
            <RoleToolGrid tools={draft.tools} onChange={tools => patch({ tools })} />
            {optimizerFeature && (
              <div style={{ marginTop: 8 }}>
                <Card>
                  <Row first title={t("settings:roles.optimizer")}
                    desc={t("settings:roles.optimizerHint")}>
                    <Toggle on={draft.optimizer === true}
                      onChange={on => setDraft(current => {
                        const next = { ...current };
                        if (on) next.optimizer = true;
                        else delete next.optimizer;
                        return next;
                      })} />
                  </Row>
                </Card>
              </div>
            )}

            <GroupTitle icon={glyph(GROUP_GLYPH.skill)}>
              {t("settings:skills.roleTitle")}</GroupTitle>
            <div style={{ fontSize: 10.5, color: "var(--tx5)", margin: "-4px 0 8px 2px",
              lineHeight: 1.6 }}>
              {t("settings:skills.roleHint")}</div>
            <RoleSkillPicker skills={ferry.skills?.skills || []}
              global={ferry.skills?.global || []} value={draft.skills}
              onChange={skills => patch({ skills })} />

            <GroupTitle icon={glyph(GROUP_GLYPH.model)}>
              {t("settings:roles.groupModel")}</GroupTitle>
            <Card>
              <Row first title={t("settings:roles.defaultModel")}
                desc={t("settings:roles.defaultModelHint")}>
                <Select value={currentModel} width={320}
                  onChange={value => {
                    if (!value) {
                      const { model: _dropped, ...rest } = draft;
                      setDraft(rest);
                      return;
                    }
                    const [provider, ...id] = value.split("/");
                    patch({ model: { provider, model: id.join("/") } });
                  }}>
                  <option value="">{t("settings:roles.followSessionModel")}</option>
                  {(ferry.models || []).map(model => (
                    <option key={`${model.provider}/${model.id}`}
                      value={`${model.provider}/${model.id}`}>
                      {model.provider_name || model.provider} · {model.name || model.id}</option>
                  ))}
                  {currentModel && !knownModel && (
                    <option value={currentModel}>
                      {t("settings:roles.modelUnavailable",
                        { provider: draft.model.provider, model: draft.model.model })}</option>)}
                </Select>
              </Row>
              <Row title={t("settings:roles.thinking")}>
                <Select value={draft.thinking || ""} width={320}
                  onChange={value => setDraft(current => {
                    const next = { ...current };
                    if (value) next.thinking = value;
                    else delete next.thinking;
                    return next;
                  })}>
                  <option value="">{t("settings:roles.followModel")}</option>
                  <option value="off">{t("settings:roles.thinkingOff")}</option>
                  <option value="low">{t("settings:roles.thinkingLow")}</option>
                  <option value="medium">{t("settings:roles.thinkingMedium")}</option>
                  <option value="high">{t("settings:roles.thinkingHigh")}</option>
                </Select>
              </Row>
            </Card>

            <GroupTitle icon={glyph(GROUP_GLYPH.security)}>
              {t("settings:roles.groupSecurity")}</GroupTitle>
            <Card>
              <Row first title={t("settings:roles.applyPolicy")}
                desc={t("settings:roles.applyPolicyHint")}>
                <span style={{ display: "inline-flex", gap: 2, padding: 2, borderRadius: 8,
                  background: "var(--fill4)", flex: "none" }}>
                  {["manual", "auto"].map(value => {
                    const on = draft.apply_policy === value;
                    return (
                      <button key={value} type="button"
                        onClick={() => patch({ apply_policy: value })}
                        style={{ border: "none", borderRadius: 6, padding: "5px 11px",
                          fontFamily: "inherit", fontSize: 11.5, fontWeight: 600, cursor: "default",
                          whiteSpace: "nowrap",
                          background: on ? "var(--surface)" : "transparent",
                          color: on ? "var(--tx1)" : "var(--tx3b)",
                          boxShadow: on ? "0 1px 2px rgba(0,0,0,.07)" : "none" }}>
                        {t(`settings:roles.${value}`)}</button>
                    );
                  })}
                </span>
              </Row>
              <div style={{ display: "flex", gap: 9, alignItems: "flex-start", padding: "11px 15px",
                borderTop: "1px solid var(--line6)", background: "var(--fill3)",
                fontSize: 10.5, lineHeight: 1.6, color: "var(--tx5)" }}>
                {glyph(INFO_GLYPH)}
                <span>{t("settings:roles.safetyNote")}</span>
              </div>
            </Card>

            {/* 删除走左栏工具条的减号;这里只剩内置角色的恢复出厂设置 */}
            {!creating && builtinSelected && (
              <div style={{ display: "flex", alignItems: "center", gap: 12, marginTop: 22,
                padding: "13px 15px", borderRadius: 12,
                border: "1px solid var(--line4)", background: "var(--fill3)" }}>
                <div style={{ flex: 1, minWidth: 0 }}>
                  <div style={{ fontSize: 12.5, fontWeight: 600, color: "var(--tx2)" }}>
                    {t("settings:roles.resetTitle")}</div>
                  <div style={{ fontSize: 11, marginTop: 2, color: "var(--tx2)",
                    opacity: .82, lineHeight: 1.55 }}>
                    {t("settings:roles.resetDesc")}</div>
                </div>
                <button className="fbtn" disabled={busy}
                  onClick={confirming ? restore : () => setConfirming(true)}
                  style={{ flex: "none", height: 30, color: "var(--tx2)",
                    borderColor: "var(--line4)", fontWeight: 600 }}>
                  {t(`settings:roles.${confirming ? "resetConfirm" : "reset"}`)}
                </button>
              </div>
            )}
        </div>
      </div>
    </div>
  );
}
