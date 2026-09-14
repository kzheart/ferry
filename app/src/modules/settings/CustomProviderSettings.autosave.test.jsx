import { act, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CustomProviderSettings } from "./CustomProviderSettings.jsx";

describe("CustomProviderSettings 自动保存", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("改 Base URL 后 800ms 内触发一次 onSave,并补全协议头", async () => {
    const onSave = vi.fn();
    const sel = { id: "cpa", name: "CPA", api: "openai-completions", base_url: "http://127.0.0.1:8318/v1" };
    render(<CustomProviderSettings sel={sel} onSave={onSave} />);
    const input = screen.getByPlaceholderText("https://api.example.com/v1");
    fireEvent.change(input, { target: { value: "cpa.example.com/v1" } });
    await act(async () => { vi.advanceTimersByTime(900); });
    expect(onSave).toHaveBeenCalledTimes(1);
    expect(onSave.mock.calls[0][0]).toMatchObject({
      provider_id: "cpa", base_url: "https://cpa.example.com/v1", models: [] });
  });
});
