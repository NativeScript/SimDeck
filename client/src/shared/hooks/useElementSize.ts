import { useEffect, useState } from "react";

import type { Size } from "../../features/viewport/types";

export function useElementSize<T extends Element>(element: T | null): Size | null {
  const [size, setSize] = useState<Size | null>(null);

  useEffect(() => {
    if (!element) {
      setSize(null);
      return;
    }

    const rect = element.getBoundingClientRect();
    if (rect.width > 0 && rect.height > 0) {
      setSize({ width: rect.width, height: rect.height });
    }

    const observer = new ResizeObserver((entries) => {
      const nextRect = entries[0]?.contentRect;
      if (!nextRect) {
        return;
      }
      setSize({ width: nextRect.width, height: nextRect.height });
    });

    observer.observe(element);
    return () => observer.disconnect();
  }, [element]);

  return size;
}
