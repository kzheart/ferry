# Ferry i18n 贡献者指南

Ferry 使用 [i18next](https://www.i18next.com/) + [react-i18next](https://react.i18next.com/) 实现界面国际化，文案按 feature 拆分成多个 namespace，方便贡献者只聚焦自己熟悉的模块。

## 当前支持的语言

- `zh-CN` — 简体中文（fallback）
- `en` — English

## 目录结构

```
app/src/locales/
  zh-CN/           # 简体中文文案
    common.json    # 通用:语言名、状态码、时间桶、相对时间、空态
    errors.json    # 引擎结构化错误码
    events.json    # 引擎结构化事件码 + 快照原因
    browser.json   # 会话详情、AI 回复编排器
    migration.json # 迁移向导、迁移历史
    snapshots.json # 快照详情、还原 toast
    onboarding.json # 欢迎页、上手引导
    settings.json  # 设置页(偏好/数据来源/软件更新)
    overlays.json  # 弹层(差异预览/确认/筛选/损耗三栏)
    app.json       # 主壳(标题栏/导航轨/toast/右键/资源栏)
  en/              # English,结构与 zh-CN 完全一致
```

## 添加一种新语言

1. 复制 `zh-CN/` 目录到新语言代码目录，例如 `ja-JP/`：
   ```
   cp -r app/src/locales/zh-CN app/src/locales/ja-JP
   ```
2. 翻译 `ja-JP/` 下每个 JSON 文件的值为目标语言。**保持 key 不变**，只翻译 value。
3. 在 `app/src/i18n/index.js` 注册新语言：
   - 顶部 import：`import jaJPCommon from "../locales/ja-JP/common.json";`（每个 namespace 都要）
   - `LOCALE_META` 数组加入 `{ code: "ja-JP", nativeName: "日本語", englishName: "Japanese" }`
   - `RESOURCES` 对象加入 `"ja-JP": { common: jaJPCommon, errors: jaJPErrors, ... }`

   设置页的语言下拉框、系统语言匹配都由 `LOCALE_META` 驱动，不需要再改界面代码。

## 翻译注意事项

### 变量插值

文案中的 `{{var}}` 是 i18next 变量占位符，**不要翻译变量名**。例如：

```json
"messages": "{{n}} 条消息"
```

英文应为：

```json
"messages": "{{n}} messages"
```

### 不要翻译的内容

以下属于机器码或领域数据，原样保留，**不要翻译**：

- 用户和 AI 消息正文、工具 input/output
- CLI stdout/stderr、文件路径、session ID、call ID
- 模型名称和第三方 label
- 原生 terminal reason
- JSON 的 key（只翻译 value）

### namespace 前缀

代码中调用 `t("browser:session.title")` 时，`browser:` 是 namespace 前缀，对应 `browser.json` 文件。翻译时只要找到对应 namespace 的 JSON 文件即可。

### 缺失 key 的行为

如果某个 key 在目标语言里缺失，i18next 会自动 fallback 到 `zh-CN`，并在开发模式（`npm run dev`）下于浏览器控制台打印 `[i18n] missing key` 警告。生产构建不会报错。

## 修改现有文案

直接编辑对应语言目录下的 JSON 文件即可。修改后热更新会自动生效（dev 模式）或下次 build 生效。

## 添加新文案

1. 在涉及的语言目录的对应 namespace JSON 里加 key。
2. 在代码里用 `t("namespace:path.to.key", { var: value })` 调用。
3. 记得同时在 `zh-CN/` 和 `en/`（以及任何已注册语言）里都加上，避免 fallback。

## 自动检测与切换

- 首次启动：读 `localStorage('ferry-settings').locale`，若为 null 则按 `navigator.language` 匹配支持列表，命中则用，否则 fallback 到 `zh-CN`。
- 用户切换：设置 → 偏好设置 → 语言，选择后立即生效（`i18n.changeLanguage` 触发所有 `useTranslation` 组件 re-render），无需重启。
- 持久化：用户选择写入 `ferry-settings.locale`，后续启动以用户选择为准。
