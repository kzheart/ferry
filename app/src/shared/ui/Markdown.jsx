// Markdown 渲染:marked 解析(GFM) + DOMPurify 消毒,输出真实 DOM,可正常选中复制
import { memo, useMemo } from "react";
import { marked } from "marked";
import DOMPurify from "dompurify";

marked.setOptions({ gfm: true, breaks: true });

// 会话内容本地可信度有限(可能含任意 HTML 片段),渲染前统一消毒
const sanitize = html => DOMPurify.sanitize(html, {
  FORBID_TAGS: ["style", "form", "input", "iframe"],
  FORBID_ATTR: ["onerror", "onclick", "onload"],
});

export default memo(function Markdown({ text }) {
  const html = useMemo(() => {
    try { return sanitize(marked.parse(String(text || ""))); }
    catch { return sanitize(String(text || "").replace(/</g, "&lt;")); }
  }, [text]);
  return <div className="fmd selectable" dangerouslySetInnerHTML={{ __html: html }} />;
})
