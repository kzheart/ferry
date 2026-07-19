import ToolIcon from "./ToolIcon.jsx";

const SIZE = { normal: 17, sm: 13 };

const Badge = ({ tool, sm }) => (
  <div className={`badge ${tool} ${sm ? "sm" : ""}`}>
    <ToolIcon tool={tool} size={sm ? SIZE.sm : SIZE.normal} />
  </div>
);

export default Badge;
