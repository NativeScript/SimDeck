import { useEffect, useRef } from "react";

import {
  isEditableTarget,
  keyCodeForKeyboardEvent,
  keyboardModifiers,
} from "./keycodes";

interface UseKeyboardInputOptions {
  enabled: boolean;
  onKey: (payload: { keyCode: number; modifiers: number }) => void;
  onToggleSoftwareKeyboard?: () => void;
}

export function useKeyboardInput({
  enabled,
  onKey,
  onToggleSoftwareKeyboard,
}: UseKeyboardInputOptions) {
  const onKeyRef = useRef(onKey);
  const onToggleSoftwareKeyboardRef = useRef(onToggleSoftwareKeyboard);

  useEffect(() => {
    onKeyRef.current = onKey;
  }, [onKey]);

  useEffect(() => {
    onToggleSoftwareKeyboardRef.current = onToggleSoftwareKeyboard;
  }, [onToggleSoftwareKeyboard]);

  useEffect(() => {
    if (!enabled) {
      return;
    }

    function handleWindowKeyDown(event: KeyboardEvent) {
      if (isEditableTarget(event.target)) {
        return;
      }
      if (isCopyShortcut(event) && hasDocumentSelection()) {
        return;
      }
      if (isSoftwareKeyboardShortcut(event)) {
        event.preventDefault();
        event.stopImmediatePropagation();
        onToggleSoftwareKeyboardRef.current?.();
        return;
      }

      const keyCode = keyCodeForKeyboardEvent(event);
      if (keyCode == null) {
        return;
      }

      event.preventDefault();
      onKeyRef.current({ keyCode, modifiers: keyboardModifiers(event) });
    }

    window.addEventListener("keydown", handleWindowKeyDown);
    return () => window.removeEventListener("keydown", handleWindowKeyDown);
  }, [enabled]);
}

function isSoftwareKeyboardShortcut(event: KeyboardEvent): boolean {
  return (
    event.key.toLowerCase() === "k" &&
    event.metaKey &&
    !event.ctrlKey &&
    !event.altKey &&
    !event.shiftKey
  );
}

function isCopyShortcut(event: KeyboardEvent): boolean {
  return (
    event.key.toLowerCase() === "c" &&
    !event.altKey &&
    !event.shiftKey &&
    (event.metaKey || event.ctrlKey)
  );
}

function hasDocumentSelection(): boolean {
  const selection = window.getSelection();
  return Boolean(selection && !selection.isCollapsed && selection.toString());
}
