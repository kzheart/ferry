import { useState } from "react";

function CopyBtn({ text, className = "copy", label = "复制", okLabel = "已复制" }) {
  const [ok, setOk] = useState(false);
  return (
    <button className={className} onClick={() => {
      navigator.clipboard.writeText(text);
      setOk(true); setTimeout(() => setOk(false), 1200);
    }}>{ok ? okLabel : label}</button>
  );
}

export default CopyBtn;
