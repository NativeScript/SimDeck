declare const global: any;
declare const require: any;
declare const process: any;

type JSONObject = Record<string, unknown>;

export interface SimDeckReactNativeInspectorOptions {
  host?: string;
  path?: string;
  port?: number;
  reconnect?: boolean;
  secure?: boolean;
}

interface InspectorRequest {
  id?: number | string | null;
  method?: string;
  params?: JSONObject | null;
}

interface InspectorError {
  code: number;
  message: string;
}

interface InspectorSocket {
  close: () => void;
  readyState: number;
  send: (payload: string) => void;
}

interface ReactNativeRuntime {
  findNodeHandle?: (componentOrHandle: unknown) => number | null;
  NativeModules?: Record<string, any>;
  Platform?: { OS?: string };
  UIManager?: {
    measure?: (
      tag: number,
      callback: (
        x: number,
        y: number,
        width: number,
        height: number,
        pageX: number,
        pageY: number,
      ) => void,
    ) => void;
  };
}

interface Fiber {
  actualDuration?: number;
  child?: Fiber | null;
  elementType?: unknown;
  key?: string | null;
  memoizedProps?: JSONObject | null;
  return?: Fiber | null;
  sibling?: Fiber | null;
  stateNode?: unknown;
  tag?: number;
  type?: unknown;
  _debugOwner?: Fiber | null;
  _debugSource?: SourceLocation | null;
  _debugStack?: unknown;
}

interface SourceLocation {
  columnNumber?: number;
  fileName?: string;
  lineNumber?: number;
}

interface FiberRoot {
  current?: Fiber | null;
}

interface HookRegistry {
  nextRendererId: number;
  roots: Map<number, Set<FiberRoot>>;
}

interface TraversalContext {
  deadline: number;
  remainingNodes: number;
}

const protocolVersion = "0.1";
const hookKey = "__REACT_DEVTOOLS_GLOBAL_HOOK__";
const hierarchyDeadlineMs = 3000;
const hierarchyNodeBudget = 3500;
const measureTimeoutMs = 2;
const commonEditableProps = [
  "accessibilityLabel",
  "accessibilityHint",
  "nativeID",
  "opacity",
  "pointerEvents",
  "style.backgroundColor",
  "style.borderColor",
  "style.opacity",
  "testID",
];

let sharedInspector: SimDeckReactNativeInspector | null = null;

export function startSimDeckReactNativeInspector(
  options: SimDeckReactNativeInspectorOptions = {},
): SimDeckReactNativeInspector {
  if (sharedInspector) {
    return sharedInspector;
  }
  sharedInspector = new SimDeckReactNativeInspector(options);
  sharedInspector.start();
  return sharedInspector;
}

export function stopSimDeckReactNativeInspector(): void {
  sharedInspector?.stop();
  sharedInspector = null;
}

export class SimDeckReactNativeInspector {
  private readonly options: Required<SimDeckReactNativeInspectorOptions>;
  private readonly ids = new WeakMap<object, string>();
  private readonly objects = new Map<string, Fiber>();
  private registry = installReactFiberHook();
  private frameCache = new Map<number, JSONObject>();
  private metadata: JSONObject | null = null;
  private nextObjectId = 1;
  private pendingFrameMeasurements = new Set<number>();
  private pollTimer: ReturnType<typeof setTimeout> | null = null;
  private polling = false;
  private pendingSourceLocations = new Set<string>();
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private socket: InspectorSocket | null = null;
  private sourceLocationCache = new Map<string, JSONObject | null>();

  constructor(options: SimDeckReactNativeInspectorOptions = {}) {
    this.options = {
      host: options.host ?? "127.0.0.1",
      path: options.path ?? "/api/inspector/connect",
      port: options.port ?? 4310,
      reconnect: options.reconnect ?? true,
      secure: options.secure ?? false,
    };
  }

  start(): void {
    this.stop();
    void this.startAsync();
  }

  stop(): void {
    if (this.reconnectTimer) {
      clearTimeout(this.reconnectTimer);
      this.reconnectTimer = null;
    }
    if (this.pollTimer) {
      clearTimeout(this.pollTimer);
      this.pollTimer = null;
    }
    this.polling = false;
    const socket = this.socket;
    this.socket = null;
    if (socket) {
      try {
        socket.close();
      } catch {
        // Ignore close failures from platform WebSocket implementations.
      }
    }
  }

  private async startAsync(): Promise<void> {
    await this.loadMetadata();
    const scheme = this.options.secure ? "wss" : "ws";
    const url = `${scheme}://${this.options.host}:${this.options.port}${this.options.path}`;
    let announced = false;
    const socket = createInspectorSocket(url, {
      onClose: () => {
        if (this.socket === socket) {
          this.socket = null;
        }
        if (this.options.reconnect) {
          this.scheduleReconnect();
        }
      },
      onError: () => {
        try {
          socket.close();
        } catch {
          // Ignore close failures from platform WebSocket implementations.
        }
      },
      onMessage: (data) => {
        void this.handleMessage(data, (payload) => {
          socket.send(JSON.stringify(payload));
        });
      },
      onOpen: () => {
        if (announced) {
          return;
        }
        announced = true;
        void this.info().then((info) => {
          socket.send(
            JSON.stringify({
              method: "Inspector.ready",
              params: info,
            }),
          );
        });
      },
    });
    this.socket = socket;
    this.startPolling();
  }

  private scheduleReconnect(): void {
    if (this.reconnectTimer) {
      return;
    }
    this.reconnectTimer = setTimeout(() => {
      this.reconnectTimer = null;
      this.start();
    }, 1000);
  }

  private startPolling(): void {
    if (this.polling) {
      return;
    }
    this.polling = true;
    this.schedulePoll(0);
  }

  private schedulePoll(delay: number): void {
    if (!this.polling) {
      return;
    }
    if (this.pollTimer) {
      clearTimeout(this.pollTimer);
    }
    this.pollTimer = setTimeout(() => {
      this.pollTimer = null;
      void this.pollCommands();
    }, delay);
  }

  private async pollCommands(): Promise<void> {
    if (!this.polling) {
      return;
    }
    try {
      const info = await this.info();
      const pid = Number(info.processIdentifier);
      const response = await fetch(
        `${this.httpBaseUrl()}/api/inspector/poll?pid=${encodeURIComponent(String(pid))}`,
      );
      if (response.status === 204) {
        this.schedulePoll(0);
        return;
      }
      if (!response.ok) {
        throw new Error(`Inspector poll failed with HTTP ${response.status}.`);
      }
      const request = (await response.json()) as InspectorRequest;
      const result = await this.executeRequest(request);
      await fetch(`${this.httpBaseUrl()}/api/inspector/response`, {
        body: JSON.stringify({
          processIdentifier: pid,
          ...result,
        }),
        headers: { "Content-Type": "application/json" },
        method: "POST",
      });
      this.schedulePoll(0);
    } catch {
      this.schedulePoll(500);
    }
  }

  private async executeRequest(request: InspectorRequest): Promise<JSONObject> {
    try {
      const result = await this.dispatch(request.method!, request.params ?? {});
      return { id: request.id ?? null, result };
    } catch (error) {
      return { id: request.id ?? null, error: inspectorError(error) };
    }
  }

  private httpBaseUrl(): string {
    const scheme = this.options.secure ? "https" : "http";
    return `${scheme}://${this.options.host}:${this.options.port}`;
  }

  private async handleMessage(
    data: string,
    respond: (payload: JSONObject) => void,
  ): Promise<void> {
    const request = JSON.parse(data) as InspectorRequest;
    if (!request.method) {
      return;
    }
    respond(await this.executeRequest(request));
  }

  private async dispatch(method: string, params: JSONObject): Promise<unknown> {
    switch (method) {
      case "Runtime.ping":
        return { ok: true, protocolVersion };
      case "Inspector.getInfo":
        return this.info();
      case "View.getHierarchy":
        return this.hierarchy(params);
      case "View.get":
        return this.getView(params);
      case "View.getProperties":
        return this.getProperties(params);
      case "View.setProperty":
        return this.setProperty(params);
      case "View.evaluateScript":
        return this.evaluateScript(params);
      case "View.listActions":
        return { id: requiredString(params, "id"), actions: [] };
      case "View.perform":
        return { ok: false, action: params.action ?? "" };
      default:
        throw new InspectorFailure(
          -32601,
          `Unknown inspector method: ${method}`,
        );
    }
  }

  private async info(): Promise<JSONObject> {
    const metadata = await this.loadMetadata();
    const roots = this.rootFibers();
    return {
      protocolVersion,
      transport: "websocket",
      processIdentifier: metadata.processIdentifier,
      bundleIdentifier: metadata.bundleIdentifier,
      bundleName: metadata.bundleName,
      displayScale: metadata.displayScale,
      screenBounds: metadata.screenBounds,
      coordinateSpace: "screen-points",
      methods: [
        "Runtime.ping",
        "Inspector.getInfo",
        "View.getHierarchy",
        "View.get",
        "View.getProperties",
        "View.setProperty",
        "View.evaluateScript",
        "View.listActions",
        "View.perform",
      ],
      appHierarchy: {
        available: roots.length > 0,
        publishedAt: new Date().toISOString(),
        source: "react-native",
      },
      reactNative: {
        available: true,
        version: reactNativeVersion(),
      },
      uikit: {
        available: false,
        propertyEditing: false,
      },
    };
  }

  private async hierarchy(params: JSONObject): Promise<JSONObject> {
    if (params.source === "uikit") {
      throw new InspectorFailure(
        -32601,
        "React Native inspector does not expose a UIKit hierarchy.",
      );
    }
    const maxDepth = optionalNumber(params.maxDepth);
    const includeHidden = Boolean(params.includeHidden);
    const roots: JSONObject[] = [];
    const visited = new WeakSet<object>();
    const context = createTraversalContext();
    for (const fiber of this.rootFibers()) {
      const node = await this.fiberNode(
        context,
        fiber,
        includeHidden,
        maxDepth,
        0,
        visited,
        null,
      );
      if (node) {
        roots.push(node);
      }
    }
    return {
      ...(await this.snapshotMetadata()),
      roots,
      source: "react-native",
    };
  }

  private async getView(params: JSONObject): Promise<JSONObject> {
    const id = requiredString(params, "id");
    const fiber = this.objects.get(id);
    if (!fiber) {
      throw new InspectorFailure(-32004, `No view was found for id ${id}.`);
    }
    const node = await this.fiberNode(
      createTraversalContext(),
      fiber,
      true,
      optionalNumber(params.maxDepth),
      0,
      new WeakSet<object>(),
      null,
    );
    if (!node) {
      throw new InspectorFailure(-32004, `No view was found for id ${id}.`);
    }
    return node;
  }

  private getProperties(params: JSONObject): JSONObject {
    const id = requiredString(params, "id");
    const fiber = this.requireFiber(id);
    return {
      id,
      className: fiberDisplayName(fiber),
      editableProperties: commonEditableProps,
      properties: propsPreview(fiber.memoizedProps ?? {}),
      reactNative: {
        tag: nativeTagForFiber(fiber),
      },
    };
  }

  private setProperty(params: JSONObject): JSONObject {
    const id = requiredString(params, "id");
    const property = requiredString(params, "property");
    const fiber = this.requireFiber(id);
    const instance = hostInstanceForFiber(fiber);
    if (!instance || typeof (instance as any).setNativeProps !== "function") {
      throw new InspectorFailure(
        -32012,
        "Selected React Native node does not expose setNativeProps.",
      );
    }
    (instance as any).setNativeProps(nativePropsPatch(property, params.value));
    return {
      ok: true,
      id,
      property,
      value: params.value ?? null,
    };
  }

  private evaluateScript(params: JSONObject): JSONObject {
    const id = requiredString(params, "id");
    const script = requiredString(params, "script");
    const fiber = this.requireFiber(id);
    const result = evaluateReactNativeScript(fiber, script);
    return {
      ok: true,
      id,
      className: fiberDisplayName(fiber),
      script,
      result: encodeValue(result),
    };
  }

  private async fiberNode(
    context: TraversalContext,
    fiber: Fiber,
    includeHidden: boolean,
    maxDepth: number | null,
    depth: number,
    visited: WeakSet<object>,
    inheritedSourceLocation: JSONObject | null,
  ): Promise<JSONObject | null> {
    await Promise.resolve();
    if (traversalExpired(context)) {
      return null;
    }
    context.remainingNodes -= 1;

    const fiberObject = fiber as object;
    if (visited.has(fiberObject)) {
      return null;
    }
    visited.add(fiberObject);

    const props = fiber.memoizedProps ?? {};
    const type = fiberDisplayName(fiber);
    const childFiberList = childFibers(fiber);
    const inspectableChildFiberList = childFiberList.filter(
      (child) => !isDevelopmentOverlayFiber(child),
    );
    if (isInactiveTabScreenFiber(props, type)) {
      return null;
    }
    const visible = includeHidden || isFiberVisible(fiber);
    const transparentWrapper =
      visible &&
      isPassThroughWrapperFiber(fiber, props, inspectableChildFiberList);
    const ownSourceLocation = await this.sourceLocationForFiber(fiber);
    const effectiveSourceLocation =
      ownSourceLocation ?? inheritedSourceLocation;

    if (!visible || transparentWrapper) {
      const children = await this.fiberChildren(
        context,
        inspectableChildFiberList,
        includeHidden,
        maxDepth,
        depth,
        visited,
        effectiveSourceLocation,
      );
      if (children.length === 1) {
        return children[0];
      }
      if (children.length > 1) {
        const tag = nativeTagForFiber(fiber);
        const frame = tag == null ? null : await this.measureNativeTag(tag);
        return this.syntheticFiberNode(
          fiber,
          type,
          props,
          effectiveSourceLocation,
          tag,
          frame,
          children,
        );
      }
      return null;
    }

    const childDepth = consumesHierarchyDepth(
      fiber,
      props,
      inspectableChildFiberList,
    )
      ? depth + 1
      : depth;
    const children =
      maxDepth == null || depth < maxDepth
        ? await this.fiberNodes(
            context,
            inspectableChildFiberList,
            includeHidden,
            maxDepth,
            childDepth,
            visited,
            effectiveSourceLocation,
          )
        : [];
    const tag = nativeTagForFiber(fiber);
    const frame = tag == null ? null : await this.measureNativeTag(tag);
    return this.syntheticFiberNode(
      fiber,
      type,
      props,
      effectiveSourceLocation,
      tag,
      frame,
      children,
    );
  }

  private async fiberNodes(
    context: TraversalContext,
    fibers: Fiber[],
    includeHidden: boolean,
    maxDepth: number | null,
    depth: number,
    visited: WeakSet<object>,
    inheritedSourceLocation: JSONObject | null,
  ): Promise<JSONObject[]> {
    const nodes: JSONObject[] = [];
    for (const child of fibers) {
      if (traversalExpired(context)) {
        break;
      }
      const node = await this.fiberNode(
        context,
        child,
        includeHidden,
        maxDepth,
        depth,
        visited,
        inheritedSourceLocation,
      );
      if (node) {
        nodes.push(node);
      }
    }
    return nodes;
  }

  private async fiberChildren(
    context: TraversalContext,
    fibers: Fiber[],
    includeHidden: boolean,
    maxDepth: number | null,
    depth: number,
    visited: WeakSet<object>,
    inheritedSourceLocation: JSONObject | null,
  ): Promise<JSONObject[]> {
    if (maxDepth != null && depth >= maxDepth) {
      return [];
    }
    return this.fiberNodes(
      context,
      fibers.filter((child) => !isDevelopmentOverlayFiber(child)),
      includeHidden,
      maxDepth,
      depth,
      visited,
      inheritedSourceLocation,
    );
  }

  private syntheticFiberNode(
    fiber: Fiber,
    type: string,
    props: JSONObject,
    sourceLocation: JSONObject | null,
    tag: number | null,
    frame: JSONObject | null,
    children: JSONObject[],
  ): JSONObject {
    const id = this.objectId(fiber);
    const effectiveFrame = frame ?? unionChildFrames(children);
    return {
      id,
      inspectorId: id,
      type,
      displayName: type,
      title: nodeTitle(type, props),
      source: "react-native",
      sourceLocation,
      sourceLocations: sourceLocation ? [sourceLocation] : [],
      frame: effectiveFrame,
      frameInScreen: effectiveFrame,
      reactNative: {
        key: fiber.key ?? null,
        tag,
        testID: stringOrNull(props.testID),
        nativeID: stringOrNull(props.nativeID),
      },
      children,
    };
  }

  private measureNativeTag(tag: number): Promise<JSONObject | null> {
    const cached = this.frameCache.get(tag);
    if (cached) {
      return Promise.resolve(cached);
    }
    if (!this.pendingFrameMeasurements.has(tag)) {
      this.pendingFrameMeasurements.add(tag);
      void measureNativeTag(tag).then((frame) => {
        this.pendingFrameMeasurements.delete(tag);
        if (frame) {
          this.frameCache.set(tag, frame);
        }
      });
    }
    return Promise.resolve(null);
  }

  private sourceLocationForFiber(fiber: Fiber): Promise<JSONObject | null> {
    const key = sourceLocationCacheKey(fiber);
    if (!key) {
      return Promise.resolve(null);
    }
    if (this.sourceLocationCache.has(key)) {
      return Promise.resolve(this.sourceLocationCache.get(key) ?? null);
    }
    const immediate = immediateSourceLocationForFiber(fiber);
    if (immediate) {
      this.sourceLocationCache.set(key, immediate);
      return Promise.resolve(immediate);
    }
    if (!this.pendingSourceLocations.has(key)) {
      this.pendingSourceLocations.add(key);
      void resolveSourceLocationForFiber(fiber).then((location) => {
        this.pendingSourceLocations.delete(key);
        this.sourceLocationCache.set(key, location);
      });
    }
    return Promise.resolve(null);
  }

  private objectId(fiber: Fiber): string {
    const object = fiber as object;
    const existing = this.ids.get(object);
    if (existing) {
      return existing;
    }
    const id = `rn:${this.nextObjectId++}`;
    this.ids.set(object, id);
    this.objects.set(id, fiber);
    return id;
  }

  private requireFiber(id: string): Fiber {
    const fiber = this.objects.get(id);
    if (!fiber) {
      throw new InspectorFailure(-32004, `No view was found for id ${id}.`);
    }
    return fiber;
  }

  private rootFibers(): Fiber[] {
    const registry = this.ensureRegistry();
    const seenRoots = new Set<FiberRoot>();
    const roots: Fiber[] = [];
    for (const root of allKnownFiberRoots(registry)) {
      if (!seenRoots.has(root)) {
        seenRoots.add(root);
        const child = root.current?.child;
        if (child) {
          roots.push(child);
        }
      }
    }
    return roots;
  }

  private ensureRegistry(): HookRegistry {
    this.registry = installReactFiberHook();
    return this.registry;
  }

  private async snapshotMetadata(): Promise<JSONObject> {
    const metadata = await this.loadMetadata();
    return {
      capturedAt: new Date().toISOString(),
      protocolVersion,
      processIdentifier: metadata.processIdentifier,
      bundleIdentifier: metadata.bundleIdentifier,
      displayScale: metadata.displayScale,
      coordinateSpace: "screen-points",
    };
  }

  private async loadMetadata(): Promise<JSONObject> {
    if (this.metadata) {
      return this.metadata;
    }
    const rn = reactNative();
    const nativeModule = rn.NativeModules?.SimDeckReactNativeInspector;
    if (!nativeModule || typeof nativeModule.getInfo !== "function") {
      throw new InspectorFailure(
        -32011,
        "SimDeckReactNativeInspector native module is not linked. Run pod install and rebuild the app.",
      );
    }
    this.metadata = (await nativeModule.getInfo()) as JSONObject;
    return this.metadata;
  }
}

function installReactFiberHook(): HookRegistry {
  const existing = global[hookKey];
  if (existing?.__simdeckRegistry) {
    return existing.__simdeckRegistry as HookRegistry;
  }

  const registry: HookRegistry = {
    nextRendererId: 1,
    roots: new Map(),
  };
  const hook = existing ?? {};
  const originalInject =
    typeof hook.inject === "function" ? hook.inject.bind(hook) : null;
  const originalCommit =
    typeof hook.onCommitFiberRoot === "function"
      ? hook.onCommitFiberRoot.bind(hook)
      : null;
  const originalUnmount =
    typeof hook.onCommitFiberUnmount === "function"
      ? hook.onCommitFiberUnmount.bind(hook)
      : null;
  const originalGetFiberRoots =
    typeof hook.getFiberRoots === "function"
      ? hook.getFiberRoots.bind(hook)
      : null;

  hook.supportsFiber = true;
  hook.renderers = hook.renderers ?? new Map();
  hook.inject = (renderer: unknown) => {
    const id = Number(originalInject?.(renderer) ?? registry.nextRendererId++);
    hook.renderers.set?.(id, renderer);
    return id;
  };
  hook.onCommitFiberRoot = (
    rendererId: number,
    root: FiberRoot,
    priorityLevel: unknown,
    didError: unknown,
  ) => {
    rootSet(registry, rendererId).add(root);
    originalCommit?.(rendererId, root, priorityLevel, didError);
  };
  hook.onCommitFiberUnmount = (rendererId: number, fiber: Fiber) => {
    originalUnmount?.(rendererId, fiber);
  };
  hook.getFiberRoots = (rendererId: number) => {
    const roots = new Set<FiberRoot>(rootSet(registry, rendererId));
    const existingRoots = originalGetFiberRoots?.(rendererId);
    if (existingRoots && typeof existingRoots[Symbol.iterator] === "function") {
      for (const root of existingRoots) {
        roots.add(root);
      }
    }
    return roots;
  };
  hook.__simdeckRegistry = registry;
  setGlobalReactDevToolsHook(hook);
  return registry;
}

function setGlobalReactDevToolsHook(hook: Record<string, any>): void {
  const descriptor = Object.getOwnPropertyDescriptor(global, hookKey);
  if (!descriptor || descriptor.writable || descriptor.set) {
    try {
      global[hookKey] = hook;
      return;
    } catch {
      // React Native 0.83 can expose this as a getter-only host property.
    }
  }

  if (descriptor?.get && descriptor.get.call(global) === hook) {
    return;
  }

  try {
    Object.defineProperty(global, hookKey, {
      configurable: true,
      enumerable: false,
      value: hook,
      writable: true,
    });
  } catch {
    // If the host marks the hook as non-configurable, mutating the existing
    // hook object above is still enough for runtimes that already created one.
  }
}

function rootSet(registry: HookRegistry, rendererId: number): Set<FiberRoot> {
  let roots = registry.roots.get(rendererId);
  if (!roots) {
    roots = new Set();
    registry.roots.set(rendererId, roots);
  }
  return roots;
}

function allKnownFiberRoots(registry: HookRegistry): FiberRoot[] {
  const roots = new Set<FiberRoot>();
  for (const rootSet of registry.roots.values()) {
    for (const root of rootSet) {
      roots.add(root);
    }
  }

  const hook = safeReactDevToolsHook();
  const renderers = hook?.renderers;
  if (
    hook &&
    typeof hook.getFiberRoots === "function" &&
    renderers &&
    typeof renderers.keys === "function"
  ) {
    for (const rendererId of renderers.keys()) {
      const rendererRoots = hook.getFiberRoots(rendererId);
      if (
        rendererRoots &&
        typeof rendererRoots[Symbol.iterator] === "function"
      ) {
        for (const root of rendererRoots) {
          roots.add(root);
        }
      }
    }
  }

  return Array.from(roots);
}

function safeReactDevToolsHook(): Record<string, any> | null {
  try {
    const hook = global[hookKey];
    return hook && typeof hook === "object" ? hook : null;
  } catch {
    return null;
  }
}

function createInspectorSocket(
  url: string,
  handlers: {
    onClose: () => void;
    onError: (error: unknown) => void;
    onMessage: (data: string) => void;
    onOpen: () => void;
  },
): InspectorSocket {
  if (typeof WebSocket !== "function") {
    throw new InspectorFailure(
      -32011,
      "No WebSocket implementation is available in this React Native runtime.",
    );
  }
  const socket = new WebSocket(url) as any;
  socket.onmessage = (event: { data: string }) => {
    handlers.onMessage(String(event.data));
  };
  socket.onclose = handlers.onClose;
  socket.onerror = handlers.onError;
  socket.onopen = handlers.onOpen;
  return socket as InspectorSocket;
}

function reactNative(): ReactNativeRuntime {
  try {
    return require("react-native") as ReactNativeRuntime;
  } catch {
    return {};
  }
}

function reactNativeVersion(): unknown {
  return (
    reactNative().NativeModules?.PlatformConstants?.reactNativeVersion ?? null
  );
}

function createTraversalContext(): TraversalContext {
  return {
    deadline: Date.now() + hierarchyDeadlineMs,
    remainingNodes: hierarchyNodeBudget,
  };
}

function traversalExpired(context: TraversalContext): boolean {
  return context.remainingNodes <= 0 || Date.now() >= context.deadline;
}

function childFibers(fiber: Fiber): Fiber[] {
  const children: Fiber[] = [];
  let child = fiber.child ?? null;
  while (child) {
    children.push(child);
    child = child.sibling ?? null;
  }
  return children;
}

function isDevelopmentOverlayFiber(fiber: Fiber): boolean {
  const type = fiberDisplayName(fiber);
  return (
    type === "DebuggingOverlay" ||
    type === "LogBoxStateSubscription" ||
    type === "_LogBoxNotificationContainer" ||
    type === "LogBoxNotification" ||
    type === "LogBoxButton" ||
    type === "PressabilityDebugView" ||
    type.startsWith("LogBox")
  );
}

function isInactiveTabScreenFiber(props: JSONObject, type: string): boolean {
  return (
    (type === "TabsScreen" || type === "RNSBottomTabsScreen") &&
    props.isFocused === false
  );
}

function fiberDisplayName(fiber: Fiber): string {
  const type = fiber.elementType ?? fiber.type;
  if (typeof type === "string") {
    return type;
  }
  if (typeof type === "function") {
    const component = type as Function & { displayName?: string };
    return component.displayName ?? component.name ?? "Component";
  }
  if (type && typeof type === "object") {
    const object = type as Record<string, any>;
    return (
      stringOrNull(object.displayName) ??
      stringOrNull(object.name) ??
      stringOrNull(object.render?.displayName) ??
      stringOrNull(object.render?.name) ??
      "Component"
    );
  }
  return fiber.tag === 6 ? "Text" : "Component";
}

async function resolveSourceLocationForFiber(
  fiber: Fiber,
): Promise<JSONObject | null> {
  const immediate = immediateSourceLocationForFiber(fiber);
  if (immediate) {
    return immediate;
  }

  const candidates = sourceLocationCandidatesForFiber(fiber);
  if (candidates.length === 0) {
    return null;
  }

  const symbolicated = await symbolicateSourceLocations(candidates);
  return bestSourceLocation(symbolicated);
}

function immediateSourceLocationForFiber(fiber: Fiber): JSONObject | null {
  const direct = bestSourceLocation(
    sourceLocationCandidatesForFiber(fiber).map(sourceLocationObject),
  );
  return direct && !isGeneratedBundleLocation(direct) ? direct : null;
}

function sourceLocationCandidatesForFiber(fiber: Fiber): SourceLocation[] {
  const candidates = [
    sourceLocationFromDisplayName(fiberDisplayName(fiber)),
    fiber._debugSource,
    fiber._debugOwner?._debugSource,
    ...sourceLocationsFromStack(fiber._debugStack),
    ...sourceLocationsFromStack(fiber._debugOwner?._debugStack),
  ];
  const seen = new Set<string>();
  return candidates.filter((candidate): candidate is SourceLocation => {
    if (!candidate?.fileName) {
      return false;
    }
    const key = `${candidate.fileName}:${candidate.lineNumber ?? ""}:${candidate.columnNumber ?? ""}`;
    if (seen.has(key)) {
      return false;
    }
    seen.add(key);
    return true;
  });
}

function sourceLocationFromDisplayName(
  displayName: string,
): SourceLocation | null {
  const match = displayName.match(/\((\.{1,2}\/.+\.[cm]?[jt]sx?)\)$/);
  if (!match) {
    return null;
  }
  return { fileName: match[1] };
}

function sourceLocationObject(
  source: SourceLocation | null | undefined,
): JSONObject | null {
  if (!source?.fileName) {
    return null;
  }
  return {
    file: source.fileName,
    line: source.lineNumber ?? null,
    column: source.columnNumber ?? null,
    kind: "react-native",
  };
}

function sourceLocationsFromStack(stackLike: unknown): SourceLocation[] {
  const stack =
    typeof stackLike === "string"
      ? stackLike
      : typeof (stackLike as { stack?: unknown } | null)?.stack === "string"
        ? String((stackLike as { stack?: string }).stack)
        : "";
  const locations: SourceLocation[] = [];
  for (const line of stack.split("\n")) {
    const match = line.match(/\(?((?:https?|file):\/\/.+):(\d+):(\d+)\)?$/);
    if (!match) {
      continue;
    }
    locations.push({
      fileName: match[1],
      lineNumber: Number(match[2]),
      columnNumber: Number(match[3]),
    });
  }
  return locations;
}

async function symbolicateSourceLocations(
  locations: SourceLocation[],
): Promise<JSONObject[]> {
  const bundleLocation = locations.find((location) =>
    location.fileName ? isMetroBundleUrl(location.fileName) : false,
  );
  if (!bundleLocation?.fileName) {
    return [];
  }

  const endpoint = metroSymbolicateUrl(bundleLocation.fileName);
  if (!endpoint) {
    return [];
  }

  try {
    const response = await fetchWithTimeout(
      endpoint,
      {
        body: JSON.stringify({
          stack: locations.map((location) => ({
            column: location.columnNumber ?? 0,
            file: location.fileName,
            lineNumber: location.lineNumber ?? 1,
            methodName: "unknown",
          })),
        }),
        headers: { "Content-Type": "application/json" },
        method: "POST",
      },
      50,
    );
    if (!response.ok) {
      return [];
    }
    const payload = (await response.json()) as {
      stack?: Array<{
        column?: number;
        file?: string;
        lineNumber?: number;
      }>;
    };
    return (payload.stack ?? [])
      .map((frame) =>
        sourceLocationObject({
          columnNumber: frame.column,
          fileName: frame.file,
          lineNumber: frame.lineNumber,
        }),
      )
      .filter((location): location is JSONObject => location != null);
  } catch {
    return [];
  }
}

function fetchWithTimeout(
  url: string,
  init: RequestInit,
  timeoutMs: number,
): Promise<Response> {
  return new Promise((resolve, reject) => {
    const timeoutId = setTimeout(
      () => reject(new Error("Symbolication timed out.")),
      timeoutMs,
    );
    fetch(url, init).then(
      (response) => {
        clearTimeout(timeoutId);
        resolve(response);
      },
      (error) => {
        clearTimeout(timeoutId);
        reject(error);
      },
    );
  });
}

function bestSourceLocation(
  locations: Array<JSONObject | null | undefined>,
): JSONObject | null {
  const usable = locations.filter((location): location is JSONObject => {
    if (!location?.file) {
      return false;
    }
    return !isGeneratedBundleLocation(location);
  });
  return (
    usable.find((location) => isAppSourceFile(String(location.file))) ??
    usable.find((location) => !isFrameworkSourceFile(String(location.file))) ??
    null
  );
}

function sourceLocationCacheKey(fiber: Fiber): string {
  return sourceLocationCandidatesForFiber(fiber)
    .map(
      (location) =>
        `${location.fileName}:${location.lineNumber ?? ""}:${location.columnNumber ?? ""}`,
    )
    .join("|");
}

function metroSymbolicateUrl(bundleUrl: string): string | null {
  try {
    const url = new URL(bundleUrl);
    url.pathname = "/symbolicate";
    url.search = "";
    url.hash = "";
    return url.toString();
  } catch {
    return null;
  }
}

function isGeneratedBundleLocation(location: JSONObject): boolean {
  return isMetroBundleUrl(String(location.file ?? ""));
}

function isMetroBundleUrl(file: string): boolean {
  return /^https?:\/\/[^/]+\/.*index\.bundle/.test(file);
}

function isAppSourceFile(file: string): boolean {
  return (
    !isFrameworkSourceFile(file) &&
    !file.includes("/node_modules/") &&
    !file.includes("/Libraries/")
  );
}

function isFrameworkSourceFile(file: string): boolean {
  return (
    file.includes("/node_modules/react/") ||
    file.includes("/node_modules/react-native/") ||
    file.includes("/node_modules/expo/") ||
    file.includes("/node_modules/expo-router/") ||
    file.includes("/node_modules/@react-navigation/")
  );
}

function consumesHierarchyDepth(
  fiber: Fiber,
  props: JSONObject,
  children: Fiber[],
): boolean {
  return !isPassThroughWrapperFiber(fiber, props, children);
}

function isPassThroughWrapperFiber(
  fiber: Fiber,
  props: JSONObject,
  children: Fiber[],
): boolean {
  if (children.length !== 1) {
    return false;
  }
  const type = fiber.elementType ?? fiber.type;
  return typeof type !== "string" && !hasMeaningfulFiberProps(props);
}

function hasMeaningfulFiberProps(props: JSONObject): boolean {
  return [
    props.accessibilityLabel,
    props.accessibilityHint,
    props.accessibilityRole,
    props.accessibilityValue,
    props.nativeID,
    props.placeholder,
    props.testID,
    stringChild(props.children),
  ].some((value) => stringOrNull(value) != null);
}

function nodeTitle(type: string, props: JSONObject): string {
  return (
    stringOrNull(props.accessibilityLabel) ??
    stringOrNull(props.testID) ??
    stringOrNull(props.nativeID) ??
    stringChild(props.children) ??
    type
  );
}

function nativeTagForFiber(fiber: Fiber): number | null {
  const instance = hostInstanceForFiber(fiber);
  const rn = reactNative();
  const direct = numericTag(instance);
  if (direct != null) {
    return direct;
  }
  try {
    const handle = rn.findNodeHandle?.(instance);
    return typeof handle === "number" && Number.isFinite(handle)
      ? handle
      : null;
  } catch {
    return null;
  }
}

function hostInstanceForFiber(fiber: Fiber): unknown {
  return fiber.stateNode ?? null;
}

function numericTag(value: unknown): number | null {
  if (typeof value === "number" && Number.isFinite(value)) {
    return value;
  }
  if (!value || typeof value !== "object") {
    return null;
  }
  const object = value as Record<string, any>;
  const candidates = [
    object._nativeTag,
    object.nativeTag,
    object.canonical?.nativeTag,
    object.node?.nativeTag,
  ];
  for (const candidate of candidates) {
    if (typeof candidate === "number" && Number.isFinite(candidate)) {
      return candidate;
    }
  }
  return null;
}

function measureNativeTag(tag: number): Promise<JSONObject | null> {
  const measure = reactNative().UIManager?.measure;
  if (typeof measure !== "function") {
    return Promise.resolve(null);
  }
  return new Promise((resolve) => {
    let settled = false;
    const finish = (frame: JSONObject | null) => {
      if (settled) {
        return;
      }
      settled = true;
      clearTimeout(timeoutId);
      resolve(frame);
    };
    const timeoutId = setTimeout(() => finish(null), measureTimeoutMs);
    try {
      measure(tag, (_x, _y, width, height, pageX, pageY) => {
        if (
          [width, height, pageX, pageY].every((value) => Number.isFinite(value))
        ) {
          finish({ x: pageX, y: pageY, width, height });
        } else {
          finish(null);
        }
      });
    } catch {
      finish(null);
    }
  });
}

function unionChildFrames(children: JSONObject[]): JSONObject | null {
  let minX = Number.POSITIVE_INFINITY;
  let minY = Number.POSITIVE_INFINITY;
  let maxX = Number.NEGATIVE_INFINITY;
  let maxY = Number.NEGATIVE_INFINITY;
  let found = false;

  for (const child of children) {
    const frame = validFrameObject(child.frame)
      ? child.frame
      : validFrameObject(child.frameInScreen)
        ? child.frameInScreen
        : null;
    if (!frame) {
      continue;
    }
    minX = Math.min(minX, frame.x);
    minY = Math.min(minY, frame.y);
    maxX = Math.max(maxX, frame.x + frame.width);
    maxY = Math.max(maxY, frame.y + frame.height);
    found = true;
  }

  if (!found) {
    return null;
  }
  return {
    x: minX,
    y: minY,
    width: maxX - minX,
    height: maxY - minY,
  };
}

function validFrameObject(value: unknown): value is {
  height: number;
  width: number;
  x: number;
  y: number;
} {
  const frame = value as {
    height?: unknown;
    width?: unknown;
    x?: unknown;
    y?: unknown;
  };
  return (
    Boolean(value) &&
    typeof value === "object" &&
    typeof frame.x === "number" &&
    typeof frame.y === "number" &&
    typeof frame.width === "number" &&
    typeof frame.height === "number" &&
    Number.isFinite(frame.x) &&
    Number.isFinite(frame.y) &&
    Number.isFinite(frame.width) &&
    Number.isFinite(frame.height) &&
    frame.width > 0 &&
    frame.height > 0
  );
}

function isFiberVisible(fiber: Fiber): boolean {
  const props = fiber.memoizedProps ?? {};
  const style = flattenStyle(props.style);
  return style.display !== "none" && style.opacity !== 0;
}

function flattenStyle(value: unknown): JSONObject {
  try {
    return flattenStyleUnsafe(value);
  } catch {
    return {};
  }
}

function flattenStyleUnsafe(value: unknown): JSONObject {
  if (!value) {
    return {};
  }
  if (Array.isArray(value)) {
    return Object.assign({}, ...value.map(flattenStyleUnsafe));
  }
  return typeof value === "object" ? (value as JSONObject) : {};
}

function propsPreview(props: JSONObject): JSONObject {
  const preview: JSONObject = {};
  for (const key of [
    "accessibilityLabel",
    "accessibilityHint",
    "nativeID",
    "pointerEvents",
    "testID",
  ]) {
    if (props[key] != null) {
      preview[key] = encodeValue(props[key]);
    }
  }
  const style = flattenStyle(props.style);
  for (const key of ["backgroundColor", "borderColor", "opacity"]) {
    if (style[key] != null) {
      preview[`style.${key}`] = encodeValue(style[key]);
    }
  }
  return preview;
}

function nativePropsPatch(property: string, value: unknown): JSONObject {
  if (!/^[A-Za-z_][A-Za-z0-9_.]*$/.test(property)) {
    throw new InspectorFailure(
      -32600,
      "View.setProperty requires a simple property or style key path.",
    );
  }
  if (property.startsWith("style.")) {
    return { style: { [property.slice("style.".length)]: value } };
  }
  return { [property]: value };
}

function evaluateReactNativeScript(fiber: Fiber, script: string): unknown {
  const trimmed = script.trim();
  if (!trimmed) {
    throw new InspectorFailure(-32600, "View.evaluateScript requires script.");
  }
  const rn = reactNative();
  const instance = hostInstanceForFiber(fiber);
  const names = ["fiber", "props", "instance", "ReactNative"];
  const values = [fiber, fiber.memoizedProps ?? {}, instance, rn];
  let expression: Function | null = null;
  try {
    expression = new Function(...names, `"use strict";\nreturn (${trimmed});`);
  } catch {
    expression = null;
  }
  try {
    if (expression) {
      return expression(...values);
    }
    return new Function(...names, `"use strict";\n${script}`)(...values);
  } catch (error) {
    throw new InspectorFailure(
      -32011,
      `React Native script failed: ${errorMessage(error)}`,
    );
  }
}

function requiredString(params: JSONObject, key: string): string {
  const value = params[key];
  if (typeof value !== "string" || value.length === 0) {
    throw new InspectorFailure(
      -32600,
      `Request params.${key} must be a non-empty string.`,
    );
  }
  return value;
}

function optionalNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function stringOrNull(value: unknown): string | null {
  return typeof value === "string" && value.length > 0 ? value : null;
}

function stringChild(value: unknown): string | null {
  if (typeof value === "string") {
    return value;
  }
  if (typeof value === "number") {
    return String(value);
  }
  if (Array.isArray(value)) {
    return value.map(stringChild).filter(Boolean).join("");
  }
  return null;
}

function encodeValue(value: unknown): unknown {
  if (
    value == null ||
    typeof value === "string" ||
    typeof value === "number" ||
    typeof value === "boolean"
  ) {
    return value ?? null;
  }
  try {
    JSON.stringify(value);
    return value;
  } catch {
    return String(value);
  }
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function inspectorError(error: unknown): InspectorError {
  if (error instanceof InspectorFailure) {
    return { code: error.code, message: error.message };
  }
  return {
    code: -32011,
    message: errorMessage(error),
  };
}

class InspectorFailure extends Error {
  constructor(
    readonly code: number,
    message: string,
  ) {
    super(message);
  }
}
