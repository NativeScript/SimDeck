import { useEffect, useRef } from "react";

import {
  isEditableTarget,
  keyCodeForKeyboardEvent,
  keyboardModifiers,
} from "./keycodes";

interface UseKeyboardInputOptions {
  enabled: boolean;
  onKey: (payload: { keyCode: number; modifiers: number }) => void;
}

export function useKeyboardInput({ enabled, onKey }: UseKeyboardInputOptions) {
  const onKeyRef = useRef(onKey);

  useEffect(() => {
    onKeyRef.current = onKey;
  }, [onKey]);

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
