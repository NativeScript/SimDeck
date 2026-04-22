import { describe, expect, it } from "vitest";

import { keyCodeForKeyboardEvent } from "./keycodes";

function keyboardEventLike(overrides: Partial<KeyboardEvent>): KeyboardEvent {
  return {
    code: "",
    key: "",
    ...overrides,
  } as KeyboardEvent;
}

describe("keyCodeForKeyboardEvent", () => {
  it("prefers the actual key value for printable characters", () => {
    const event = keyboardEventLike({
      code: "KeyQ",
      key: "a",
    });

    expect(keyCodeForKeyboardEvent(event)).toBe(0);
  });

  it("maps shifted printable characters to their underlying key", () => {
    const event = keyboardEventLike({
      code: "Slash",
      key: "?",
    });

    expect(keyCodeForKeyboardEvent(event)).toBe(44);
  });

  it("falls back to the physical code for control keys", () => {
    const event = keyboardEventLike({
      code: "ArrowLeft",
      key: "ArrowLeft",
    });

    expect(keyCodeForKeyboardEvent(event)).toBe(123);
  });
});
