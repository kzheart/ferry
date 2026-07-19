import Spin from "./Spin.jsx";

function Steps({ items }) {
  return (
    <div className="steps">
      {items.map(([st, t, d], i) => (
        <div className="step" key={i}>
          <div className="rail">
            <span className={`ico ${st}`}>
              {st === "done" ? "✓" : st === "run" ? <Spin /> : st === "fail" ? "✕" : ""}
            </span>
            {i < items.length - 1 && <span className="line" />}
          </div>
          <div className="txt"><div className="t">{t}</div><div className="d">{d}</div></div>
        </div>
      ))}
    </div>
  );
}

export default Steps;
