import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { ACCENT } from "../../shared/ui/toolDisplay.js";

export function Guide({ step, steps, onGo, onFinish }) {
  const { t } = useTranslation();
  const [box, setBox] = useState(null);
  const [card, setCard] = useState(null);
  const config = steps[step - 1];

  useEffect(() => {
    setBox(null);
    const root = document.querySelector("[data-ferry-win]");
    if (!root || !config) return;

    const measure = () => {
      const target = document.querySelector(
        `[data-guide="${config.target}"]`,
      );
      if (!target) return;
      const windowRect = root.getBoundingClientRect();
      const targetRect = target.getBoundingClientRect();
      const padding = 8;
      const windowWidth = windowRect.width;
      const windowHeight = windowRect.height;
      const cardWidth = 324;
      const left = targetRect.left - windowRect.left - padding;
      const top = Math.max(8, targetRect.top - windowRect.top - padding);
      const width = targetRect.width + padding * 2;
      const height = targetRect.height + padding * 2;
      let cardLeft;
      let cardTop;
      if (config.side === "right") {
        cardLeft = left + width + 18;
        cardTop = top;
      } else if (config.side === "top") {
        cardLeft = left;
        cardTop = top - 198;
      } else {
        cardLeft = left + width - cardWidth;
        cardTop = top + height + 16;
      }
      cardLeft = Math.min(
        Math.max(12, cardLeft),
        windowWidth - cardWidth - 12,
      );
      cardTop = Math.min(Math.max(12, cardTop), windowHeight - 212);
      setBox({
        left,
        top,
        width,
        height,
        windowWidth,
        windowHeight,
      });
      setCard({ left: cardLeft, top: cardTop });
    };

    // 步骤可能刚切换模块,锚点要等新视图渲染完;轮询等待,始终等不到
    // (比如没有选中的会话)就不高亮,把说明卡居中展示
    let timer = null;
    let tries = 0;
    const attempt = () => {
      const target = document.querySelector(
        `[data-guide="${config.target}"]`,
      );
      if (!target) {
        tries += 1;
        if (tries < 12) {
          timer = setTimeout(attempt, 120);
          return;
        }
        const rect = root.getBoundingClientRect();
        setCard({
          left: Math.max(12, (rect.width - 324) / 2),
          top: Math.max(12, rect.height / 2 - 120),
        });
        return;
      }
      if (config.scroll) {
        const scroller = document.querySelector("[data-guide-scroll]");
        if (scroller) {
          const targetRect = target.getBoundingClientRect();
          const scrollRect = scroller.getBoundingClientRect();
          scroller.scrollTop += targetRect.top - scrollRect.top - 170;
        }
        timer = setTimeout(measure, 80);
        return;
      }
      measure();
    };
    timer = setTimeout(attempt, 30);
    return () => clearTimeout(timer);
  }, [step, config]);

  if (!config) return null;

  const dim = "var(--dim)";
  return (
    <div style={{ position: "absolute", inset: 0, zIndex: 50 }}>
      {box && (
        <>
          <div style={{
            position: "absolute",
            left: 0,
            top: 0,
            width: box.windowWidth,
            height: box.top,
            background: dim,
          }} />
          <div style={{
            position: "absolute",
            left: 0,
            top: box.top + box.height,
            width: box.windowWidth,
            height: Math.max(
              0,
              box.windowHeight - box.top - box.height,
            ),
            background: dim,
          }} />
          <div style={{
            position: "absolute",
            left: 0,
            top: box.top,
            width: Math.max(0, box.left),
            height: box.height,
            background: dim,
          }} />
          <div style={{
            position: "absolute",
            left: box.left + box.width,
            top: box.top,
            width: Math.max(
              0,
              box.windowWidth - box.left - box.width,
            ),
            height: box.height,
            background: dim,
          }} />
          <div style={{
            position: "absolute",
            left: box.left,
            top: box.top,
            width: box.width,
            height: box.height,
            borderRadius: 8,
            outline: `2px solid ${ACCENT}`,
            boxShadow: "0 0 0 4px var(--ring)",
            pointerEvents: "none",
            transition: "all .26s cubic-bezier(.2,.7,.3,1)",
          }} />
        </>
      )}
      <div style={{
        position: "absolute",
        left: card?.left ?? -9999,
        top: card?.top ?? 0,
        width: 324,
        background: "var(--bg)",
        borderRadius: 10,
        boxShadow: "var(--shadow-menu)",
        padding: "16px 18px 14px",
        transition: "all .26s cubic-bezier(.2,.7,.3,1)",
      }}>
        <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
          <span style={{
            fontSize: 11,
            fontWeight: 600,
            color: ACCENT,
            letterSpacing: ".03em",
          }}>
            {step} / {steps.length}
          </span>
          <div style={{ display: "flex", gap: 4, marginLeft: 2 }}>
            {steps.map((_, index) => index + 1).map(index => (
              <span
                key={index}
                style={{
                  width: 16,
                  height: 3,
                  borderRadius: 2,
                  background: index <= step ? ACCENT : "var(--dots)",
                }}
              />
            ))}
          </div>
          <span style={{ flex: 1 }} />
          <a onClick={onFinish} style={{ fontSize: 11, color: "var(--tx5)" }}>
            {t("onboarding:guide.skip")}
          </a>
        </div>
        <div style={{
          fontSize: 14,
          fontWeight: 600,
          marginTop: 11,
          letterSpacing: "-.01em",
        }}>
          {t(config.titleKey)}
        </div>
        <div style={{
          fontSize: 12,
          color: "var(--tx3)",
          lineHeight: 1.55,
          marginTop: 6,
        }}>
          {t(config.bodyKey)}
        </div>
        <div style={{
          display: "flex",
          alignItems: "center",
          gap: 9,
          marginTop: 15,
        }}>
          {step > 1 && (
            <button
              className="fbtn"
              style={{ height: 31, fontSize: 12 }}
              onClick={() => onGo(step - 1)}
            >
              {t("onboarding:guide.back")}
            </button>
          )}
          <span style={{ flex: 1 }} />
          <button
            className="fbtn-primary"
            style={{ height: 31, padding: "0 16px", fontSize: 12 }}
            onClick={() => step >= steps.length
              ? onFinish()
              : onGo(step + 1)}
          >
            {step >= steps.length
              ? t("onboarding:guide.start")
              : t("onboarding:guide.next")}
          </button>
        </div>
      </div>
    </div>
  );
}
