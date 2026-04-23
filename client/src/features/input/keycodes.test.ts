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
  it("prefers the actual key value for printable characters as a USB HID usage", () => {
    const event = keyboardEventLike({
      code: "KeyQ",
      key: "a",
    });

    expect(keyCodeForKeyboardEvent(event)).toBe(4);
  });

  it("maps shifted printable characters to their underlying key", () => {
    const event = keyboardEventLike({
      code: "Slash",
      key: "?",
    });

    expect(keyCodeForKeyboardEvent(event)).toBe(56);
  });

  it("falls back to the physical code for control keys", () => {
    const event = keyboardEventLike({
      code: "ArrowLeft",
      key: "ArrowLeft",
    });

    expect(keyCodeForKeyboardEvent(event)).toBe(80);
  });

  it("maps escape to the USB HID escape usage instead of macOS virtual keycode", () => {
    const event = keyboardEventLike({
      code: "Escape",
      key: "Escape",
    });

    expect(keyCodeForKeyboardEvent(event)).toBe(41);
  });

  it("maps h to the USB HID h usage instead of macOS virtual keycode", () => {
    const event = keyboardEventLike({
      code: "KeyH",
      key: "h",
    });

    expect(keyCodeForKeyboardEvent(event)).toBe(11);
  });
});
