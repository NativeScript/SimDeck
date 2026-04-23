import type { StreamConnectTarget, WorkerToMainMessage } from "./streamTypes";

export function buildStreamTarget(udid: string): StreamConnectTarget {
  return { udid };
}

interface StreamClientBackend {
  attachCanvas(canvasElement: HTMLCanvasElement): void;
  clear(): void;
  connect(target: StreamConnectTarget): void;
  destroy(): void;
  disconnect(): void;
  resize(width: number, height: number, devicePixelRatio: number): void;
}

class WorkerStreamClient implements StreamClientBackend {
  private readonly worker: Worker;

  constructor(onMessage: (message: WorkerToMainMessage) => void) {
    this.worker = new Worker(
      new URL("../../workers/simulatorStream.worker.ts", import.meta.url),
      {
        type: "module",
      },
    );
    this.worker.onmessage = (event: MessageEvent<WorkerToMainMessage>) => {
      onMessage(event.data);
    };
  }

  attachCanvas(canvasElement: HTMLCanvasElement) {
    const offscreenCanvas = canvasElement.transferControlToOffscreen();
    this.worker.postMessage(
      { type: "attach-canvas", canvas: offscreenCanvas },
      [offscreenCanvas],
    );
  }

  connect(target: StreamConnectTarget) {
    this.worker.postMessage({ type: "connect", target });
  }

  disconnect() {
    this.worker.postMessage({ type: "disconnect" });
  }

  clear() {
    this.worker.postMessage({ type: "clear" });
  }

  resize(width: number, height: number, devicePixelRatio: number) {
    this.worker.postMessage({
      type: "resize",
      width,
      height,
      devicePixelRatio,
    });
  }

  destroy() {
    this.worker.terminate();
  }
}

export class StreamWorkerClient {
  private readonly onMessage: (message: WorkerToMainMessage) => void;
  private backend: StreamClientBackend | null = null;
  private attachedCanvas = false;
  private disposed = false;

  constructor(onMessage: (message: WorkerToMainMessage) => void) {
    this.onMessage = onMessage;
  }

  attachCanvas(canvasElement: HTMLCanvasElement) {
    if (this.attachedCanvas) {
      return;
    }

    this.backend = this.createBackend(canvasElement);
    this.backend.attachCanvas(canvasElement);
    this.attachedCanvas = true;
  }

  connect(target: StreamConnectTarget) {
    this.backend?.connect(target);
  }

  disconnect() {
    this.backend?.disconnect();
  }

  clear() {
    this.backend?.clear();
  }

  resize(width: number, height: number, devicePixelRatio: number) {
    this.backend?.resize(width, height, devicePixelRatio);
  }

  destroy() {
    if (this.disposed) {
      return;
    }
    this.disposed = true;
    this.backend?.destroy();
    this.backend = null;
  }

  private createBackend(canvasElement: HTMLCanvasElement): StreamClientBackend {
    void canvasElement;
    return new WorkerStreamClient(this.onMessage);
  }
}
