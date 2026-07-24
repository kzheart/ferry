import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { ACCENT } from "../../shared/ui/toolDisplay.js";

const GUIDE_STEPS = [
  {
    target: "rail",
    side: "right",
    titleKey: "onboarding:guide.step1Title",
    bodyKey: "onboarding:guide.step1Body",
  },
  {
    target: "search",
    side: "right",
    titleKey: "onboarding:guide.step2Title",
    bodyKey: "onboarding:guide.step2Body",
  },
  {
    target: "scope",
    side: "top",
    scroll: true,
    titleKey: "onboarding:guide.step3Title",
    bodyKey: "onboarding:guide.step3Body",
  },
];

export function Guide({ step, onGo, onFinish }) {
  const { t } = useTranslation();
  const [box, setBox] = useState(null);
  const [card, setCard] = useState(null);
  const config = GUIDE_STEPS[step - 1];

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

    let delay = 30;
    if (config.scroll) {
      const scroller = document.querySelector("[data-guide-scroll]");
      const target = document.querySelector(
        `[data-guide="${config.target}"]`,
      );
      if (scroller && target) {
        const targetRect = target.getBoundingClientRect();
        const scrollRect = scroller.getBoundingClientRect();
        scroller.scrollTop += targetRect.top - scrollRect.top - 170;
        delay = 80;
      }
    }
    const timer = setTimeout(measure, delay);
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
            fontWeight: 700,
            color: ACCENT,
            letterSpacing: ".03em",
          }}>
            {step} / {GUIDE_STEPS.length}
          </span>
          <div style={{ display: "flex", gap: 4, marginLeft: 2 }}>
            {GUIDE_STEPS.map((_, index) => index + 1).map(index => (
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
          fontWeight: 650,
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
            onClick={() => step >= GUIDE_STEPS.length
              ? onFinish()
              : onGo(step + 1)}
          >
            {step >= GUIDE_STEPS.length
              ? t("onboarding:guide.start")
              : t("onboarding:guide.next")}
          </button>
        </div>
      </div>
    </div>
  );
}
