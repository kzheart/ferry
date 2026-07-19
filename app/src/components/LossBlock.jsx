function LossBlock({ loss }) {
  const total = loss.native + loss.degrade + loss.drop || 1;
  return (
    <>
      <div style={{ fontSize: 13, fontWeight: 700 }}>
        损耗报告 · 共 {loss.native + loss.degrade + loss.drop} 项
        {loss.degrade + loss.drop === 0 &&
          <span className="tag ok" style={{ marginLeft: 8 }}>无损</span>}
      </div>
      <div className="loss-bar">
        <div style={{ flex: loss.native / total, background: "#17A886" }} />
        <div style={{ flex: loss.degrade / total, background: "#D99A2B" }} />
        <div style={{ flex: loss.drop / total, background: "#CB5A52" }} />
      </div>
      <div className="loss-cards">
        {[["native", "原生映射", loss.native, "直接对应目标结构", "#0F9D7A", "#0B7A5E"],
          ["degrade", "降级为文本", loss.degrade, "保留内容,失去结构", "#A66A00", "#8A5A00"],
          ["drop", "丢弃", loss.drop, "无法安全迁移", "#C2413A", "#9E332D"]]
          .map(([k, lbl, n, d, c1, c2]) => (
            <div className="card loss-card" key={k}>
              <div className="lbl" style={{ color: c1 }}><span className={`sq ${k}`} />{lbl}</div>
              <div className="n" style={{ color: c2 }}>{n}</div>
              <div className="d">{d}</div>
            </div>
          ))}
      </div>
      {(loss.degrade_details.length > 0 || loss.drop_details.length > 0) && (
        <div className="card" style={{ padding: "13px 15px", fontSize: 12, color: "#5A6672",
          display: "flex", flexDirection: "column", gap: 7 }}>
          {loss.degrade_details.map((x, i) =>
            <div key={"d" + i}><span className="sq degrade" style={{ marginRight: 7 }} />{x}</div>)}
          {loss.drop_details.map((x, i) =>
            <div key={"x" + i}><span className="sq drop" style={{ marginRight: 7 }} />{x}</div>)}
        </div>
      )}
    </>
  );
}

export default LossBlock;
