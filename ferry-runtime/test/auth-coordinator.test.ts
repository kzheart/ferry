import { describe, expect, it } from "vitest";
import {
  AuthCoordinator,
  type AuthCoordinatorEvent,
} from "../src/providers/auth-coordinator.js";

async function until(events: AuthCoordinatorEvent[], type: string) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    const event = events.find((item) => item.type === type);
    if (event) return event;
    await new Promise((resolve) => setTimeout(resolve, 1));
  }
  throw new Error(`event ${type} was not emitted`);
}

describe("AuthCoordinator", () => {
  it("starts the login only after returning its id", async () => {
    let startReturned = false;
    let loginStarted = false;
    const coordinator = new AuthCoordinator(
      async (_provider, _type, interaction) => {
        expect(startReturned).toBe(true);
        loginStarted = true;
        await interaction.prompt({
          type: "manual_code",
          message: "Paste code",
        });
        return { type: "oauth", access: "a", refresh: "r", expires: 1 };
      },
      () => {},
    );

    const login = coordinator.start("openai-codex", "oauth");
    expect(loginStarted).toBe(false);
    startReturned = true;
    await new Promise<void>((resolve) => setImmediate(resolve));
    expect(loginStarted).toBe(true);
    coordinator.cancel(login.login_id);
  });

  it("bridges prompts without echoing secret responses", async () => {
    const events: AuthCoordinatorEvent[] = [];
    let nextId = 0;
    const coordinator = new AuthCoordinator(
      async (_provider, _type, interaction) => {
        interaction.notify({ type: "progress", message: "starting" });
        const key = await interaction.prompt({
          type: "secret",
          message: "API key",
        });
        expect(key).toBe("plain-secret");
        return { type: "api_key", key };
      },
      (event) => events.push(event),
      () => `auth-${++nextId}`,
    );
    const login = coordinator.start("openai", "api_key");
    const prompt = await until(events, "auth.prompt");
    coordinator.respond(
      login.login_id,
      prompt.payload.prompt_id as string,
      "plain-secret",
    );
    await until(events, "auth.completed");

    expect(JSON.stringify(events)).not.toContain("plain-secret");
    expect(events.map((event) => event.type)).toEqual([
      "auth.event",
      "auth.prompt",
      "auth.completed",
    ]);
  });

  it("cancels a pending OAuth prompt and rejects duplicate provider logins", async () => {
    const events: AuthCoordinatorEvent[] = [];
    let nextId = 0;
    const coordinator = new AuthCoordinator(
      async (_provider, _type, interaction) => {
        await interaction.prompt({
          type: "manual_code",
          message: "Paste code",
        });
        return { type: "oauth", access: "a", refresh: "r", expires: 1 };
      },
      (event) => events.push(event),
      () => `auth-${++nextId}`,
    );
    const login = coordinator.start("anthropic", "oauth");
    expect(() => coordinator.start("anthropic", "oauth")).toThrow(
      "already in progress",
    );
    await until(events, "auth.prompt");
    coordinator.cancel(login.login_id);
    await until(events, "auth.cancelled");
    expect(coordinator.isProviderActive("anthropic")).toBe(false);
  });
});
