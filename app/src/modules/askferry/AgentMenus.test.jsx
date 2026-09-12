import { createRef } from "react";
import { fireEvent, render, screen } from "@testing-library/react";
import { expect, test, vi } from "vitest";
import { FerryRuntimeProvider } from "../../shared/capabilities/ferryRuntime.jsx";
import { ModeMenu, ModelMenu } from "./AgentMenus.jsx";

function fixture(
  Menu,
  props = {},
  rect = { left: 100, top: 500, bottom: 526 },
) {
  const anchorRef = createRef();
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(
    function () {
      if (this.getAttribute("role") === "menu")
        return { height: 200, width: 268 };
      return { ...rect, width: 120, height: 26, right: rect.left + 120 };
    },
  );
  const ferry = {
    models: [
      {
        provider: "openai",
        id: "gpt",
        name: "GPT test model",
        provider_name: "OpenAI",
        reasoning: true,
      },
    ],
    selectModel: vi.fn(async () => {}),
    reportError: vi.fn(),
  };
  const onClose = vi.fn();
  const onManage = vi.fn();
  const rendered = render(
    <FerryRuntimeProvider value={ferry}>
      <div style={{ overflow: "hidden", transform: "translateY(0)" }}>
        <button ref={anchorRef}>trigger</button>
        <Menu
          anchorRef={anchorRef}
          onClose={onClose}
          onManage={onManage}
          health={{ provider: "openai", model: "gpt", thinking: "low" }}
          {...props}
        />
      </div>
    </FerryRuntimeProvider>,
  );
  return { ...rendered, ferry, onClose, onManage, anchorRef };
}

test("model menu portals above its anchor outside clipping and transformed ancestors", () => {
  const { container } = fixture(ModelMenu);
  const menu = screen.getByRole("menu");
  expect(menu.parentElement).toBe(document.body);
  expect(container.contains(menu)).toBe(false);
  expect(menu.style.position).toBe("fixed");
  expect(menu.style.visibility).toBe("visible");
  expect(menu.style.left).toBe("100px");
  expect(menu.style.top).toBe("292px");
  expect(screen.getByRole("menuitem", { name: /GPT test model/ }).tagName).toBe(
    "BUTTON",
  );
});

test("menu flips below a high anchor and stays within the right viewport edge", () => {
  fixture(ModelMenu, {}, { left: window.innerWidth - 20, top: 24, bottom: 50 });
  const menu = screen.getByRole("menu");
  expect(menu.style.top).toBe("58px");
  expect(menu.style.left).toBe(`${window.innerWidth - 268 - 8}px`);
});

test("model selection and reasoning panel remain clickable after portaling", () => {
  const { ferry } = fixture(ModelMenu);
  fireEvent.click(screen.getByRole("menuitem", { name: /GPT test model/ }));
  expect(ferry.selectModel).toHaveBeenCalledWith("openai", "gpt", "low");
  fireEvent.click(
    screen.getByRole("menuitem", { name: /askferry:model.effort/ }),
  );
  fireEvent.click(
    screen.getByRole("menuitem", { name: "askferry:model.effort_high" }),
  );
  expect(ferry.selectModel).toHaveBeenCalledWith("openai", "gpt", "high");
});

test("mode options expose buttons and Escape restores focus to the trigger", () => {
  const onPick = vi.fn();
  const { onClose, anchorRef } = fixture(ModeMenu, { mode: "manual", onPick });
  const menu = screen.getByRole("menu");
  const items = screen.getAllByRole("menuitem");
  expect(document.activeElement).toBe(items[0]);
  fireEvent.keyDown(menu, { key: "ArrowDown" });
  expect(document.activeElement).toBe(items[1]);
  fireEvent.click(items[1]);
  expect(onPick).toHaveBeenCalledWith("auto");
  fireEvent.keyDown(menu, { key: "Escape" });
  expect(onClose).toHaveBeenCalled();
  expect(document.activeElement).toBe(anchorRef.current);
});
