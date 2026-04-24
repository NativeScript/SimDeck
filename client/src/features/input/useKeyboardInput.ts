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
