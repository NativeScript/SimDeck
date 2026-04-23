import { Application, View } from "@nativescript/core";

declare const NSBundle: any;
declare const NSClassFromString: any;
declare const NSProcessInfo: any;
declare const NSStringFromClass: any;
declare const NSString: any;
declare const NSURL: any;
declare const NSURLSession: any;
declare const NSURLSessionWebSocketMessage: any;
declare const NSURLSessionWebSocketTask: any;
declare const UIApplication: any;
declare const UIColor: any;
declare const UIScreen: any;
declare const UIControl: any;
declare const UITextField: any;
declare const UITextView: any;
declare const UIScrollView: any;
declare const UISwitch: any;
declare const UISlider: any;
declare const UISegmentedControl: any;
declare const CGRectMake: any;
declare const CGPointMake: any;
declare const CGSizeMake: any;
declare const UIEdgeInsetsMake: any;
declare const require: any;
declare const WebSocket: any | undefined;

type JSONObject = Record<string, unknown>;

export interface XcodeCanvasInspectorOptions {
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

interface InspectorSocketHandlers {
  onClose: () => void;
  onError: (error: unknown) => void;
  onMessage: (data: string) => void;
  onOpen: () => void;
}

const protocolVersion = "0.1";
const controlEventTouchUpInside = 1 << 6;
const controlEventValueChanged = 1 << 12;
const controlEventPrimaryActionTriggered = 1 << 13;
const controlEventEditingChanged = 1 << 17;
const angularSourceLocationAttribute = "data-ng-source-location";
const uikitLastScript = Symbol("xcodeCanvasLastUIKitScript");
const defaultEditableProperties = [
  "alpha",
  "backgroundColor",
  "clipsToBounds",
  "hidden",
  "opaque",
  "text",
  "tintColor",
  "userInteractionEnabled",
];

let sharedInspector: XcodeCanvasNativeScriptInspector | null = null;
const nativeScriptDebugAttributes = Symbol("xcodeCanvasDebugAttributes");
const fallbackUIKitLastScripts = new WeakMap<object, string>();
let angularSourceLocationCaptureInstalled = false;

export function startXcodeCanvasInspector(
  options: XcodeCanvasInspectorOptions = {},
): XcodeCanvasNativeScriptInspector {
  installAngularDebugAttributeShim();
  installAngularSourceLocationCaptureShim();
  if (sharedInspector) {
    return sharedInspector;
  }
  sharedInspector = new XcodeCanvasNativeScriptInspector(options);
  sharedInspector.start();
  return sharedInspector;
}

export function stopXcodeCanvasInspector(): void {
  sharedInspector?.stop();
  sharedInspector = null;
}

function installAngularDebugAttributeShim(): void {
  const prototype = View.prototype as any;
  if (typeof prototype.hasAttribute !== "function") {
    prototype.hasAttribute = function hasAttribute(name: string): boolean {
      if (debugAttributeStore(this).has(name)) {
        return true;
      }
      return safeCall(
        () =>
          Object.prototype.hasOwnProperty.call(this, name) ||
          this[name] != null,
        false,
      );
    };
  }
  if (typeof prototype.getAttribute !== "function") {
    prototype.getAttribute = function getAttribute(
      name: string,
    ): string | null {
      const attributes = debugAttributeStore(this);
      if (attributes.has(name)) {
        return attributes.get(name) ?? null;
      }
      return safeCall(() => stringValue(this[name]) || null, null);
    };
  }
  if (typeof prototype.setAttribute !== "function") {
    prototype.setAttribute = function setAttribute(
      name: string,
      value: unknown,
    ): void {
      debugAttributeStore(this).set(name, stringValue(value));
    };
  }
  if (typeof prototype.removeAttribute !== "function") {
    prototype.removeAttribute = function removeAttribute(name: string): void {
      debugAttributeStore(this).delete(name);
    };
  }
}

function debugAttributeStore(node: any): Map<string, string> {
  let attributes = node[nativeScriptDebugAttributes] as
    | Map<string, string>
    | undefined;
  if (!attributes) {
    attributes = new Map<string, string>();
    Object.defineProperty(node, nativeScriptDebugAttributes, {
      configurable: false,
      enumerable: false,
      value: attributes,
    });
  }
  return attributes;
}

function installAngularSourceLocationCaptureShim(): void {
  if (angularSourceLocationCaptureInstalled) {
    return;
  }

  const angular = safeCall(() => {
    if (typeof require !== "function") {
      return null;
    }
    return require("@nativescript/angular");
  }, null) as any;
  const viewUtilPrototype = angular?.ɵViewUtil?.ViewUtil?.prototype;
  if (
    !viewUtilPrototype ||
    typeof viewUtilPrototype.setProperty !== "function"
  ) {
    return;
  }

  angularSourceLocationCaptureInstalled = true;
  const originalSetProperty = viewUtilPrototype.setProperty;
  viewUtilPrototype.setProperty = function setPropertyWithSourceLocationCapture(
    view: any,
    attributeName: string,
    value: unknown,
    namespace?: string,
  ) {
    if (attributeName === angularSourceLocationAttribute) {
      debugAttributeStore(view).set(attributeName, stringValue(value));
      return;
    }
    return originalSetProperty.call(
      this,
      view,
      attributeName,
      value,
      namespace,
    );
  };
}

export class XcodeCanvasNativeScriptInspector {
  private readonly options: Required<XcodeCanvasInspectorOptions>;
  private socket: InspectorSocket | null = null;
  private pollTimer: ReturnType<typeof setTimeout> | null = null;
  private polling = false;
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private nextObjectId = 1;
  private readonly ids = new WeakMap<object, string>();
  private readonly nativeScriptViewsByNativeView = new WeakMap<object, View>();
  private readonly objects = new Map<string, any>();
  private readonly uikitScriptsById = new Map<string, string>();

  constructor(options: XcodeCanvasInspectorOptions = {}) {
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
        this.handleMessage(data, (payload) => {
          socket.send(JSON.stringify(payload));
        });
      },
      onOpen: () => {
        if (announced) {
          return;
        }
        announced = true;
        socket.send(
          JSON.stringify({
            method: "Inspector.ready",
            params: this.info(),
          }),
        );
      },
    });
    this.socket = socket;
    if (socket.readyState === 1) {
      socket.send(
        JSON.stringify({
          method: "Inspector.ready",
          params: this.info(),
        }),
      );
      announced = true;
    }
    this.startPolling();
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
      this.pollCommands();
    }, delay);
  }

  private pollCommands(): void {
    if (!this.polling) {
      return;
    }

    const pid = Number(NSProcessInfo.processInfo.processIdentifier);
    const pollUrl = `${this.httpBaseUrl()}/api/inspector/poll?pid=${encodeURIComponent(String(pid))}`;
    fetch(pollUrl)
      .then((response) => {
        if (response.status === 204) {
          return null;
        }
        if (!response.ok) {
          throw new Error(
            `Inspector poll failed with HTTP ${response.status}.`,
          );
        }
        return response.json() as Promise<InspectorRequest>;
      })
      .then((request) => {
        if (!request) {
          return null;
        }
        return this.executePolledRequest(request);
      })
      .then((response) => {
        if (!response) {
          return null;
        }
        return fetch(`${this.httpBaseUrl()}/api/inspector/response`, {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify({
            processIdentifier: pid,
            ...response,
          }),
        });
      })
      .then(() => {
        this.schedulePoll(0);
      })
      .catch(() => {
        this.schedulePoll(500);
      });
  }

  private executePolledRequest(request: InspectorRequest): Promise<JSONObject> {
    return new Promise((resolve) => {
      dispatchMain(() => {
        try {
          const result = this.dispatch(request.method!, request.params ?? {});
          resolve({ id: request.id ?? null, result });
        } catch (error) {
          resolve({
            id: request.id ?? null,
            error: inspectorError(error),
          });
        }
      });
    });
  }

  private httpBaseUrl(): string {
    const scheme = this.options.secure ? "https" : "http";
    return `${scheme}://${this.options.host}:${this.options.port}`;
  }

  private handleMessage(
    data: string,
    respond: (payload: JSONObject) => void,
  ): void {
    const request = JSON.parse(data) as InspectorRequest;
    if (!request.method) {
      return;
    }

    dispatchMain(() => {
      try {
        const result = this.dispatch(request.method!, request.params ?? {});
        respond({ id: request.id ?? null, result });
      } catch (error) {
        respond({
          id: request.id ?? null,
          error: inspectorError(error),
        });
      }
    });
  }

  private send(payload: JSONObject): void {
    if (!this.socket || this.socket.readyState !== 1) {
      return;
    }
    this.socket.send(JSON.stringify(payload));
  }

  private dispatch(method: string, params: JSONObject): unknown {
    switch (method) {
      case "Runtime.ping":
        return { ok: true, protocolVersion };
      case "Inspector.getInfo":
        return this.info();
      case "View.getHierarchy":
        return this.hierarchy(params);
      case "View.get":
        return this.getView(params);
      case "View.listActions":
        return this.listActions(params);
      case "View.perform":
        return this.perform(params);
      case "View.getProperties":
        return this.getProperties(params);
      case "View.setProperty":
        return this.setProperty(params);
      case "View.evaluateScript":
        return this.evaluateScript(params);
      default:
        throw new InspectorFailure(
          -32601,
          `Unknown inspector method: ${method}`,
        );
    }
  }

  private info(): JSONObject {
    const bundle = NSBundle.mainBundle;
    const rootView = safeCall(() => Application.getRootView(), null);
    return {
      protocolVersion,
      transport: "websocket",
      processIdentifier: Number(NSProcessInfo.processInfo.processIdentifier),
      bundleIdentifier: stringValue(bundle.bundleIdentifier),
      bundleName: stringValue(
        bundle.objectForInfoDictionaryKey("CFBundleName"),
      ),
      displayScale: numberValue(UIScreen.mainScreen.scale, 1),
      screenBounds: rectValue(UIScreen.mainScreen.bounds),
      coordinateSpace: "screen-points",
      methods: [
        "Runtime.ping",
        "Inspector.getInfo",
        "View.getHierarchy",
        "View.get",
        "View.listActions",
        "View.perform",
        "View.getProperties",
        "View.setProperty",
        "View.evaluateScript",
      ],
      appHierarchy: {
        source: "nativescript",
        available: Boolean(rootView),
        publishedAt: new Date().toISOString(),
      },
      nativeScript: {
        available: Boolean(rootView),
        runtime: "NativeScript",
      },
      uikit: {
        available: true,
        propertyEditing: true,
      },
    };
  }

  private hierarchy(params: JSONObject): JSONObject {
    this.reindexNativeScriptViews();
    const maxDepth = optionalNumber(params.maxDepth);
    const includeHidden = Boolean(params.includeHidden);
    if (params.source === "uikit") {
      return {
        ...this.snapshotMetadata("in-app-inspector"),
        roots: this.windows()
          .filter((window) => includeHidden || isVisible(window))
          .map((window) => this.uikitNode(window, includeHidden, maxDepth, 0)),
      };
    }

    const rootView = safeCall(
      () => Application.getRootView(),
      null,
    ) as View | null;
    if (!rootView) {
      return { ...this.snapshotMetadata("nativescript"), roots: [] };
    }

    return {
      ...this.snapshotMetadata("nativescript"),
      roots: [this.nativeScriptNode(rootView, includeHidden, maxDepth, 0)],
    };
  }

  private getView(params: JSONObject): JSONObject {
    const id = requiredString(params, "id");
    const target = this.objects.get(id);
    if (!target) {
      throw new InspectorFailure(-32004, `No view was found for id ${id}.`);
    }
    const maxDepth = optionalNumber(params.maxDepth);
    if (isNativeScriptView(target)) {
      return this.nativeScriptNode(target, true, maxDepth, 0);
    }
    return this.uikitNode(target, true, maxDepth, 0);
  }

  private listActions(params: JSONObject): JSONObject {
    const id = requiredString(params, "id");
    const target = this.requireObject(id);
    return {
      id,
      actions: actionsFor(target),
    };
  }

  private perform(params: JSONObject): JSONObject {
    const id = requiredString(params, "id");
    const action = requiredString(params, "action");
    const target = nativeTarget(this.requireObject(id));
    switch (action) {
      case "describe":
        return { ok: true, id, actions: actionsFor(target) };
      case "setText":
        setText(target, stringValue(params.value));
        return { ok: true, action };
      case "setValue":
        setControlValue(target, params.value);
        return { ok: true, action };
      case "toggle":
        setControlValue(target, !Boolean(read(target, "on")));
        return { ok: true, action };
      case "focus":
        return { ok: Boolean(call(target, "becomeFirstResponder")), action };
      case "resignFirstResponder":
        return { ok: Boolean(call(target, "resignFirstResponder")), action };
      case "accessibilityActivate":
        return { ok: Boolean(call(target, "accessibilityActivate")), action };
      case "tap":
        tap(target);
        return { ok: true, action };
      case "scrollBy":
      case "scrollTo":
        return scroll(target, params, action === "scrollBy");
      default:
        throw new InspectorFailure(
          -32010,
          `Unsupported view action: ${action}`,
        );
    }
  }

  private getProperties(params: JSONObject): JSONObject {
    const id = requiredString(params, "id");
    const target = nativeTarget(this.requireObject(id));
    const properties: JSONObject = {};
    for (const property of editablePropertiesFor(target)) {
      properties[property] = encodeValue(read(target, property));
    }
    return {
      id,
      className: className(target),
      editableProperties: editablePropertiesFor(target),
      properties,
    };
  }

  private setProperty(params: JSONObject): JSONObject {
    const id = requiredString(params, "id");
    const property = requiredString(params, "property");
    if (!/^[A-Za-z_][A-Za-z0-9_.]*$/.test(property)) {
      throw new InspectorFailure(
        -32600,
        "View.setProperty requires a simple property or key path.",
      );
    }

    const target = nativeTarget(this.requireObject(id));
    const value = decodeValue(params.value);
    if (property.includes(".") && hasMethod(target, "setValueForKeyPath")) {
      target.setValueForKeyPath(value, property);
    } else if (hasMethod(target, "setValueForKey")) {
      target.setValueForKey(value, property);
    } else {
      target[property] = value;
    }
    call(target, "setNeedsLayout");
    call(target, "setNeedsDisplay");

    return {
      ok: true,
      id,
      property,
      value: encodeValue(read(target, property.split(".")[0])),
    };
  }

  private evaluateScript(params: JSONObject): JSONObject {
    const id = requiredString(params, "id");
    const script = requiredString(params, "script");
    const target = nativeTarget(this.requireObject(id));
    const result = evaluateUIKitScript(target, script);
    setLastUIKitScript(target, script);
    this.uikitScriptsById.set(id, script);
    call(target, "setNeedsLayout");
    call(target, "setNeedsDisplay");
    return {
      ok: true,
      id,
      className: className(target),
      script,
      result: encodeValue(result),
    };
  }

  private nativeScriptNode(
    view: View,
    includeHidden: boolean,
    maxDepth: number | null,
    depth: number,
  ): JSONObject {
    const nativeView = nativeViewFor(view);
    const children: JSONObject[] = [];
    if (maxDepth == null || depth < maxDepth) {
      safeCall(() => {
        view.eachChildView((child: View) => {
          if (includeHidden || isNativeScriptVisible(child)) {
            children.push(
              this.nativeScriptNode(child, includeHidden, maxDepth, depth + 1),
            );
          }
          return true;
        });
      }, undefined);
    }

    const id = this.objectId("ns", view);
    const uikitId = nativeView ? this.objectId("view", nativeView) : null;
    return {
      id,
      inspectorId: id,
      type: stringValue((view as any).typeName) || constructorName(view),
      title: nativeScriptTitle(view),
      source: "nativescript",
      sourceLocation: sourceLocationForView(view),
      frame: nativeView ? frameInScreen(nativeView) : null,
      nativeScript: {
        id: stringValue((view as any).id),
        className: stringValue((view as any).className),
      },
      uikit: nativeView
        ? {
            id: uikitId,
            className: className(nativeView),
            script: this.uikitScriptFor(uikitId, nativeView),
          }
        : null,
      uikitId,
      children,
    };
  }

  private uikitNode(
    view: any,
    includeHidden: boolean,
    maxDepth: number | null,
    depth: number,
  ): JSONObject {
    const nativeClassName = className(view);
    const nativeScriptView = this.nativeScriptViewsByNativeView.get(view);
    const nativeScriptType = nativeScriptView
      ? nativeScriptViewType(nativeScriptView)
      : "";
    const children =
      maxDepth != null && depth >= maxDepth
        ? []
        : nsArray(read(view, "subviews"))
            .filter((child) => includeHidden || isVisible(child))
            .map((child) =>
              this.uikitNode(child, includeHidden, maxDepth, depth + 1),
            );

    const id = this.objectId("view", view);
    return {
      id,
      type: nativeScriptType || nativeClassName,
      displayName: nativeScriptType || nativeClassName,
      className: nativeClassName,
      moduleName: moduleName(nativeClassName),
      debugDescription: stringValue(view),
      uikitScript: this.uikitScriptInfo(id, view),
      sourceLocation: nativeScriptView
        ? sourceLocationForView(nativeScriptView)
        : null,
      frame: rectValue(read(view, "frame")),
      bounds: rectValue(read(view, "bounds")),
      frameInScreen: frameInScreen(view),
      center: pointValue(read(view, "center")),
      transform: stringValue(read(view, "transform")),
      alpha: numberValue(read(view, "alpha"), 1),
      isHidden: Boolean(read(view, "hidden") ?? read(view, "isHidden")),
      isOpaque: Boolean(read(view, "opaque") ?? read(view, "isOpaque")),
      clipsToBounds: Boolean(read(view, "clipsToBounds")),
      isUserInteractionEnabled: Boolean(read(view, "userInteractionEnabled")),
      backgroundColor: colorValue(read(view, "backgroundColor")),
      tintColor: colorValue(read(view, "tintColor")),
      accessibility: accessibilityInfo(view),
      viewController: null,
      text: textValue(view),
      placeholder: stringValue(read(view, "placeholder")),
      imageName: null,
      nativeScript: nativeScriptView
        ? {
            id: stringValue((nativeScriptView as any).id),
            className: stringValue((nativeScriptView as any).className),
            type: nativeScriptType,
            inspectorId: this.objectId("ns", nativeScriptView),
          }
        : null,
      scroll: scrollInfo(view),
      control: controlInfo(view),
      children,
    };
  }

  private objectId(prefix: string, object: object): string {
    const existing = this.ids.get(object);
    if (existing) {
      return existing;
    }
    const id = `${prefix}:${this.nextObjectId++}`;
    this.ids.set(object, id);
    this.objects.set(id, object);
    return id;
  }

  private requireObject(id: string): any {
    const target = this.objects.get(id);
    if (!target) {
      throw new InspectorFailure(-32004, `No view was found for id ${id}.`);
    }
    return target;
  }

  private uikitScriptFor(id: string | null, view: any): string {
    return (id ? this.uikitScriptsById.get(id) : "") || lastUIKitScript(view);
  }

  private uikitScriptInfo(id: string, view: any): JSONObject | null {
    const script = this.uikitScriptFor(id, view);
    return script ? { script } : null;
  }

  private snapshotMetadata(source: string): JSONObject {
    return {
      protocolVersion,
      capturedAt: new Date().toISOString(),
      processIdentifier: Number(NSProcessInfo.processInfo.processIdentifier),
      bundleIdentifier: stringValue(NSBundle.mainBundle.bundleIdentifier),
      displayScale: numberValue(UIScreen.mainScreen.scale, 1),
      coordinateSpace: "screen-points",
      source,
    };
  }

  private windows(): any[] {
    const application = UIApplication.sharedApplication;
    const connectedScenes = read(application, "connectedScenes");
    const windows: any[] = [];
    for (const scene of nsSet(connectedScenes)) {
      for (const window of nsArray(read(scene, "windows"))) {
        windows.push(window);
      }
    }
    if (windows.length > 0) {
      return windows;
    }
    return nsArray(read(application, "windows"));
  }

  private reindexNativeScriptViews(): void {
    const rootView = safeCall(
      () => Application.getRootView(),
      null,
    ) as View | null;
    if (!rootView) {
      return;
    }

    this.indexNativeScriptView(rootView, new Set<View>());
    safeCall(() => {
      const modalViews = (rootView as any)._getRootModalViews?.();
      for (const modalView of Array.isArray(modalViews) ? modalViews : []) {
        this.indexNativeScriptView(modalView, new Set<View>());
      }
    }, undefined);
  }

  private indexNativeScriptView(view: View, visited: Set<View>): void {
    if (!view || visited.has(view)) {
      return;
    }
    visited.add(view);

    const nativeView = nativeViewFor(view);
    if (nativeView && typeof nativeView === "object") {
      this.nativeScriptViewsByNativeView.set(nativeView, view);
    }

    safeCall(() => {
      view.eachChildView((child: View) => {
        this.indexNativeScriptView(child, visited);
        return true;
      });
    }, undefined);
  }
}

function createInspectorSocket(
  url: string,
  handlers: InspectorSocketHandlers,
): InspectorSocket {
  if (typeof WebSocket === "function") {
    const socket = new WebSocket(url) as any;
    socket.onmessage = (event: { data: string }) => {
      handlers.onMessage(String(event.data));
    };
    socket.onclose = handlers.onClose;
    socket.onerror = handlers.onError;
    socket.onopen = handlers.onOpen;
    return socket as InspectorSocket;
  }

  if (typeof NSURLSession !== "undefined") {
    return createNSURLSessionWebSocket(url, handlers);
  }

  throw new InspectorFailure(
    -32011,
    "No WebSocket implementation is available in this NativeScript runtime.",
  );
}

function createNSURLSessionWebSocket(
  url: string,
  handlers: InspectorSocketHandlers,
): InspectorSocket {
  const nsUrl = NSURL.URLWithString(url);
  // Touch the subclass so NativeScript loads its metadata before wrapping the returned task.
  safeCall(() => NSURLSessionWebSocketTask.prototype, null);
  const task = NSURLSession.sharedSession.webSocketTaskWithURL(nsUrl);
  const sendMessage = nativeMethod(
    task,
    NSURLSessionWebSocketTask,
    "sendMessageCompletionHandler",
  );
  const receiveMessage = nativeMethod(
    task,
    NSURLSessionWebSocketTask,
    "receiveMessageWithCompletionHandler",
  );
  let closed = false;

  const socket: InspectorSocket = {
    close: () => {
      if (closed) {
        return;
      }
      closed = true;
      task.cancelWithCloseCodeReason(1000, null);
      handlers.onClose();
    },
    readyState: 1,
    send: (payload: string) => {
      const message =
        NSURLSessionWebSocketMessage.alloc().initWithString(payload);
      sendMessage(message, (error: unknown) => {
        if (error && !closed) {
          handlers.onError(error);
        }
      });
    },
  };

  const receive = () => {
    if (closed) {
      return;
    }
    receiveMessage((message: unknown, error: unknown) => {
      if (closed) {
        return;
      }
      if (error) {
        closed = true;
        handlers.onError(error);
        handlers.onClose();
        return;
      }
      const text = nsWebSocketMessageText(message);
      if (text != null) {
        handlers.onMessage(text);
      }
      receive();
    });
  };

  task.resume();
  receive();
  return socket;
}

function nativeMethod(
  target: any,
  klass: any,
  methodName: string,
): (...args: unknown[]) => unknown {
  if (typeof target?.[methodName] === "function") {
    return target[methodName].bind(target);
  }
  const inherited = klass?.prototype?.[methodName];
  if (typeof inherited === "function") {
    return inherited.bind(target);
  }
  throw new InspectorFailure(
    -32011,
    `${className(target)} does not expose ${methodName}.`,
  );
}

function nsWebSocketMessageText(message: any): string | null {
  if (!message) {
    return null;
  }
  if (message.string != null) {
    return String(message.string);
  }
  if (message.data != null) {
    return String(NSString.alloc().initWithDataEncoding(message.data, 4));
  }
  return null;
}

class InspectorFailure extends Error {
  constructor(
    readonly code: number,
    message: string,
  ) {
    super(message);
  }
}

function dispatchMain(work: () => void): void {
  if (typeof setTimeout === "function") {
    setTimeout(work, 0);
  } else {
    work();
  }
}

function inspectorError(error: unknown): InspectorError {
  if (error instanceof InspectorFailure) {
    return { code: error.code, message: error.message };
  }
  return {
    code: -32011,
    message: error instanceof Error ? error.message : String(error),
  };
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

function nativeTarget(target: any): any {
  return isNativeScriptView(target)
    ? (nativeViewFor(target) ?? target)
    : target;
}

function nativeViewFor(view: any): any | null {
  return (
    view?.nativeViewProtected ??
    view?.nativeView ??
    view?.ios ??
    view?._nativeView ??
    null
  );
}

function isNativeScriptView(value: any): value is View {
  return Boolean(value && typeof value.eachChildView === "function");
}

function isNativeScriptVisible(view: any): boolean {
  return read(view, "visibility") !== "collapse";
}

function isVisible(view: any): boolean {
  const hidden = Boolean(read(view, "hidden") ?? read(view, "isHidden"));
  const alpha = numberValue(read(view, "alpha"), 1);
  return !hidden && alpha > 0;
}

function nsArray(value: any): any[] {
  if (!value) {
    return [];
  }
  if (Array.isArray(value)) {
    return value;
  }
  const count = Number(value.count ?? 0);
  const result: any[] = [];
  for (let index = 0; index < count; index += 1) {
    result.push(value.objectAtIndex(index));
  }
  return result;
}

function nsSet(value: any): any[] {
  if (!value) {
    return [];
  }
  if (typeof read(value, "allObjects") !== "undefined") {
    return nsArray(read(value, "allObjects"));
  }
  return nsArray(value);
}

function read(target: any, property: string): any {
  if (!target) {
    return null;
  }
  const value = target[property];
  if (typeof value === "function") {
    try {
      return value.call(target);
    } catch {
      return value;
    }
  }
  return value;
}

function call(target: any, method: string, ...args: unknown[]): unknown {
  if (!target || typeof target[method] !== "function") {
    return null;
  }
  return target[method](...args);
}

function hasMethod(target: any, method: string): boolean {
  return Boolean(target && typeof target[method] === "function");
}

function safeCall<T>(work: () => T, fallback: T): T {
  try {
    return work();
  } catch {
    return fallback;
  }
}

function evaluateUIKitScript(view: any, script: string): unknown {
  const trimmed = script.trim();
  if (!trimmed) {
    throw new InspectorFailure(-32600, "View.evaluateScript requires script.");
  }

  const scope = uikitScriptScope();
  const names = Object.keys(scope);
  const values = names.map((name) => scope[name]);
  let expression: Function | null = null;
  try {
    expression = new Function(
      "view",
      ...names,
      `"use strict";\nreturn (${trimmed});`,
    );
  } catch {
    expression = null;
  }

  try {
    if (expression) {
      return expression(view, ...values);
    }
    const statement = new Function(
      "view",
      ...names,
      `"use strict";\n${script}`,
    );
    return statement(view, ...values);
  } catch (error) {
    throw new InspectorFailure(
      -32011,
      `UIKit script failed: ${errorMessage(error)}`,
    );
  }
}

function uikitScriptScope(): Record<string, unknown> {
  return {
    Application,
    UIApplication,
    UIColor: uikitClassProxy(UIColor),
    UIScreen,
  };
}

function uikitClassProxy(klass: any): any {
  if (!klass || typeof Proxy === "undefined") {
    return klass;
  }
  return new Proxy(klass, {
    get(target, property) {
      const value = target[property as keyof typeof target];
      if (
        typeof property === "string" &&
        typeof value === "function" &&
        isZeroArgumentUIKitFactory(property)
      ) {
        return value.call(target);
      }
      return typeof value === "function" ? value.bind(target) : value;
    },
  });
}

function isZeroArgumentUIKitFactory(property: string): boolean {
  return property.endsWith("Color") && !property.startsWith("colorWith");
}

function setLastUIKitScript(view: any, script: string): void {
  safeCall(() => {
    Object.defineProperty(view, uikitLastScript, {
      configurable: true,
      enumerable: false,
      value: script,
      writable: true,
    });
  }, undefined);
  if (view && typeof view === "object") {
    fallbackUIKitLastScripts.set(view, script);
  }
}

function lastUIKitScript(view: any): string {
  const direct = safeCall(() => stringValue(view?.[uikitLastScript]), "");
  if (direct) {
    return direct;
  }
  return view && typeof view === "object"
    ? (fallbackUIKitLastScripts.get(view) ?? "")
    : "";
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function className(object: any): string {
  const name = safeCall(() => {
    const klass =
      typeof object?.class === "function" ? object.class() : object?.class;
    return klass ? NSStringFromClass(klass) : constructorName(object);
  }, constructorName(object));
  if (name !== "UIView" && name !== "NSObject" && name !== "Object") {
    return name;
  }
  return debugClassName(object) || name;
}

function debugClassName(object: any): string {
  const description = stringValue(object);
  const match = /^<([^:>]+)[:>]/.exec(description);
  return match?.[1] ?? "";
}

function constructorName(object: any): string {
  return object?.constructor?.name ?? "Object";
}

function moduleName(name: string): string | null {
  return name.includes(".") ? name.split(".")[0] : null;
}

function stringValue(value: unknown): string {
  if (value == null) {
    return "";
  }
  try {
    return String(value);
  } catch {
    return typeof value === "object" ? constructorName(value) : "";
  }
}

function numberValue(value: unknown, fallback = 0): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

function finiteNumber(value: unknown): number | null {
  return typeof value === "number" && Number.isFinite(value) ? value : null;
}

function rectValue(rect: any): JSONObject | null {
  if (!rect) {
    return null;
  }
  const x = finiteNumber(rect.origin?.x ?? rect.x);
  const y = finiteNumber(rect.origin?.y ?? rect.y);
  const width = finiteNumber(rect.size?.width ?? rect.width);
  const height = finiteNumber(rect.size?.height ?? rect.height);
  if (x == null || y == null || width == null || height == null) {
    return null;
  }
  return {
    x,
    y,
    width,
    height,
  };
}

function pointValue(point: any): JSONObject | null {
  if (!point) {
    return null;
  }
  const x = finiteNumber(point.x);
  const y = finiteNumber(point.y);
  if (x == null || y == null) {
    return null;
  }
  return {
    x,
    y,
  };
}

function sizeValue(size: any): JSONObject | null {
  if (!size) {
    return null;
  }
  const width = finiteNumber(size.width);
  const height = finiteNumber(size.height);
  if (width == null || height == null) {
    return null;
  }
  return {
    width,
    height,
  };
}

function insetsValue(insets: any): JSONObject | null {
  if (!insets) {
    return null;
  }
  const top = finiteNumber(insets.top);
  const left = finiteNumber(insets.left);
  const bottom = finiteNumber(insets.bottom);
  const right = finiteNumber(insets.right);
  if (top == null || left == null || bottom == null || right == null) {
    return null;
  }
  return {
    top,
    left,
    bottom,
    right,
  };
}

function frameInScreen(view: any): JSONObject | null {
  return safeCall(
    () => rectValue(view.convertRectToView(read(view, "bounds"), null)),
    rectValue(read(view, "frame")),
  );
}

function colorValue(color: any): JSONObject | null {
  if (!color) {
    return null;
  }
  return safeCall(() => {
    if (!isKindOf(color, "UIColor")) {
      return null;
    }
    const description = stringValue(color);
    if (!description) {
      return null;
    }
    return { description };
  }, null);
}

function accessibilityInfo(view: any): JSONObject {
  return {
    identifier: stringValue(read(view, "accessibilityIdentifier")),
    label: stringValue(read(view, "accessibilityLabel")),
    value: stringValue(read(view, "accessibilityValue")),
    hint: stringValue(read(view, "accessibilityHint")),
    traits: numberValue(read(view, "accessibilityTraits")),
    isElement: Boolean(read(view, "isAccessibilityElement")),
  };
}

function textValue(view: any): string {
  if (
    isKindOf(view, "UILabel") ||
    isKindOf(view, "UITextField") ||
    isKindOf(view, "UITextView")
  ) {
    return stringValue(read(view, "text"));
  }
  if (isKindOf(view, "UIButton")) {
    return stringValue(call(view, "titleForState", read(view, "state")));
  }
  if (isKindOf(view, "UISegmentedControl")) {
    const index = numberValue(read(view, "selectedSegmentIndex"), -1);
    return index >= 0
      ? stringValue(call(view, "titleForSegmentAtIndex", index))
      : "";
  }
  return "";
}

function scrollInfo(view: any): JSONObject | null {
  if (!isKindOf(view, "UIScrollView")) {
    return null;
  }
  return {
    contentOffset: pointValue(read(view, "contentOffset")),
    contentSize: sizeValue(read(view, "contentSize")),
    adjustedContentInset: insetsValue(read(view, "adjustedContentInset")),
    isScrollEnabled: Boolean(read(view, "scrollEnabled")),
  };
}

function controlInfo(view: any): JSONObject | null {
  if (!isKindOf(view, "UIControl")) {
    return null;
  }
  return {
    controlState: numberValue(read(view, "state")),
    controlEvents: numberValue(read(view, "allControlEvents")),
    isSelected: Boolean(read(view, "selected")),
    isHighlighted: Boolean(read(view, "highlighted")),
    actions: actionsFor(view),
  };
}

function actionsFor(target: any): string[] {
  const native = nativeTarget(target);
  const actions = new Set(["describe", "getProperties", "setProperty"]);
  if (
    isKindOf(native, "UIControl") ||
    Boolean(read(native, "isAccessibilityElement"))
  ) {
    actions.add("tap");
  }
  if (Boolean(read(native, "canBecomeFirstResponder"))) {
    actions.add("focus");
  }
  if (Boolean(read(native, "isFirstResponder"))) {
    actions.add("resignFirstResponder");
  }
  if (isKindOf(native, "UITextField") || isKindOf(native, "UITextView")) {
    actions.add("setText");
  }
  if (isKindOf(native, "UISwitch")) {
    actions.add("toggle");
    actions.add("setValue");
  }
  if (isKindOf(native, "UISlider") || isKindOf(native, "UISegmentedControl")) {
    actions.add("setValue");
  }
  if (isKindOf(native, "UIScrollView")) {
    actions.add("scrollBy");
    actions.add("scrollTo");
  }
  if (Boolean(read(native, "isAccessibilityElement"))) {
    actions.add("accessibilityActivate");
  }
  return [...actions].sort();
}

function editablePropertiesFor(target: any): string[] {
  const properties = new Set(defaultEditableProperties);
  if (
    isKindOf(target, "UILabel") ||
    isKindOf(target, "UITextField") ||
    isKindOf(target, "UITextView")
  ) {
    properties.add("text");
    properties.add("textColor");
  }
  if (isKindOf(target, "UIScrollView")) {
    properties.add("contentOffset");
  }
  if (isKindOf(target, "UISwitch")) {
    properties.add("on");
  }
  if (isKindOf(target, "UISlider")) {
    properties.add("value");
  }
  if (isKindOf(target, "UISegmentedControl")) {
    properties.add("selectedSegmentIndex");
  }
  return [...properties].sort();
}

function isKindOf(object: any, classNameValue: string): boolean {
  const klass = safeCall(() => NSClassFromString(classNameValue), null);
  return Boolean(
    klass &&
    object &&
    typeof object.isKindOfClass === "function" &&
    object.isKindOfClass(klass),
  );
}

function nativeScriptViewType(view: View): string {
  return stringValue((view as any).typeName) || constructorName(view);
}

function sourceLocationForView(view: any): JSONObject | null {
  const raw = safeCall(
    () => view.getAttribute?.(angularSourceLocationAttribute),
    null,
  );
  const value = stringValue(raw);
  if (!value) {
    return null;
  }
  const separator = value.lastIndexOf("@");
  if (separator <= 0) {
    return { file: value };
  }
  const location: JSONObject = { file: value.slice(0, separator) };
  for (const part of value.slice(separator + 1).split(",")) {
    const [key, rawNumber] = part.split(":");
    const parsed = Number(rawNumber);
    if (!Number.isFinite(parsed)) {
      continue;
    }
    if (key === "o") {
      location.offset = parsed;
    } else if (key === "l") {
      location.line = parsed;
    } else if (key === "c") {
      location.column = parsed;
    }
  }
  return location;
}

function nativeScriptTitle(view: View): string {
  const anyView = view as any;
  return (
    stringValue(anyView.text) ||
    stringValue(anyView.title) ||
    stringValue(anyView.id) ||
    stringValue(anyView.typeName) ||
    constructorName(view)
  );
}

function tap(target: any): void {
  if (isKindOf(target, "UISwitch")) {
    target.setOnAnimated(!Boolean(read(target, "on")), true);
    sendActions(target, controlEventValueChanged);
    return;
  }
  if (isKindOf(target, "UIControl")) {
    sendActions(target, controlEventPrimaryActionTriggered);
    sendActions(target, controlEventTouchUpInside);
    return;
  }
  if (!call(target, "accessibilityActivate")) {
    call(target, "becomeFirstResponder");
  }
}

function setText(target: any, value: string): void {
  if (!(isKindOf(target, "UITextField") || isKindOf(target, "UITextView"))) {
    throw new InspectorFailure(
      -32011,
      "setText is only supported for UITextField and UITextView.",
    );
  }
  target.text = value;
  if (isKindOf(target, "UITextField")) {
    sendActions(target, controlEventEditingChanged);
  }
}

function setControlValue(target: any, value: unknown): void {
  if (isKindOf(target, "UISwitch")) {
    target.setOnAnimated(Boolean(value), true);
    sendActions(target, controlEventValueChanged);
    return;
  }
  if (isKindOf(target, "UISlider")) {
    target.value = numberValue(value);
    sendActions(target, controlEventValueChanged);
    return;
  }
  if (isKindOf(target, "UISegmentedControl")) {
    target.selectedSegmentIndex = numberValue(value);
    sendActions(target, controlEventValueChanged);
    return;
  }
  throw new InspectorFailure(
    -32011,
    "setValue is only supported for UISwitch, UISlider, and UISegmentedControl.",
  );
}

function sendActions(target: any, event: number): void {
  if (typeof target.sendActionsForControlEvents === "function") {
    target.sendActionsForControlEvents(event);
  }
}

function scroll(
  target: any,
  params: JSONObject,
  relative: boolean,
): JSONObject {
  if (!isKindOf(target, "UIScrollView")) {
    throw new InspectorFailure(
      -32011,
      "scroll actions are only supported for UIScrollView.",
    );
  }
  const current = read(target, "contentOffset");
  const x = numberValue(params.x);
  const y = numberValue(params.y);
  const next = relative
    ? CGPointMake(numberValue(current.x) + x, numberValue(current.y) + y)
    : CGPointMake(x, y);
  target.setContentOffsetAnimated(next, Boolean(params.animated));
  return {
    ok: true,
    action: relative ? "scrollBy" : "scrollTo",
    contentOffset: pointValue(read(target, "contentOffset")),
  };
}

function encodeValue(value: any): unknown {
  if (value == null) {
    return null;
  }
  if (
    typeof value === "string" ||
    typeof value === "number" ||
    typeof value === "boolean"
  ) {
    return value;
  }
  return (
    rectValue(value) ??
    pointValue(value) ??
    sizeValue(value) ??
    insetsValue(value) ??
    colorValue(value) ??
    stringValue(value)
  );
}

function decodeValue(value: unknown): unknown {
  if (!value || typeof value !== "object") {
    return value;
  }
  const object = value as JSONObject;
  switch (object.$type) {
    case "UIColor":
      return colorFromObject(object);
    case "CGRect":
      return CGRectMake(
        numberValue(object.x),
        numberValue(object.y),
        numberValue(object.width),
        numberValue(object.height),
      );
    case "CGPoint":
      return CGPointMake(numberValue(object.x), numberValue(object.y));
    case "CGSize":
      return CGSizeMake(numberValue(object.width), numberValue(object.height));
    case "UIEdgeInsets":
      return UIEdgeInsetsMake(
        numberValue(object.top),
        numberValue(object.left),
        numberValue(object.bottom),
        numberValue(object.right),
      );
    default:
      if ("hex" in object) {
        return colorFromObject(object);
      }
      return value;
  }
}

function colorFromObject(object: JSONObject): any {
  if (typeof object.hex === "string") {
    const hexValue = object.hex.replace(/^#/, "");
    const red = parseInt(hexValue.slice(0, 2), 16) / 255;
    const green = parseInt(hexValue.slice(2, 4), 16) / 255;
    const blue = parseInt(hexValue.slice(4, 6), 16) / 255;
    const alpha =
      hexValue.length >= 8 ? parseInt(hexValue.slice(6, 8), 16) / 255 : 1;
    return UIColor.colorWithRedGreenBlueAlpha(red, green, blue, alpha);
  }
  return UIColor.colorWithRedGreenBlueAlpha(
    numberValue(object.red),
    numberValue(object.green),
    numberValue(object.blue),
    numberValue(object.alpha, 1),
  );
}
