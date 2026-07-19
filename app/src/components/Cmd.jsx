import CopyBtn from "./CopyBtn.jsx";

const Cmd = ({ text }) => (
  <div className="cmd mono">
    <span className="c selectable">{text}</span>
    <CopyBtn text={text} />
  </div>
);

export default Cmd;
