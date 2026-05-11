import { useEffect, useState } from "react";

const PANEL_TRANSITION_MS = 240;

export function usePanelPresence(visible: boolean): {
  isPresent: boolean;
  panelState: "closed" | "open";
} {
  const [isPresent, setIsPresent] = useState(visible);
  const [panelState, setPanelState] = useState<"closed" | "open">(
    visible ? "open" : "closed",
  );

  useEffect(() => {
    let frame = 0;
    let timeout = 0;

    if (visible) {
      setIsPresent(true);
      setPanelState("closed");
      frame = window.requestAnimationFrame(() => setPanelState("open"));
    } else {
      setPanelState("closed");
      timeout = window.setTimeout(
        () => setIsPresent(false),
        PANEL_TRANSITION_MS,
      );
    }

    return () => {
      if (frame) {
        window.cancelAnimationFrame(frame);
      }
      if (timeout) {
        window.clearTimeout(timeout);
      }
    };
  }, [visible]);

  return { isPresent, panelState };
}
