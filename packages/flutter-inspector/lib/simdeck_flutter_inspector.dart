import 'dart:async';
import 'dart:convert';
import 'dart:io' as io;
import 'dart:math' as math;
import 'dart:ui' show FlutterView;

import 'package:flutter/foundation.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

const String _protocolVersion = '0.1';
const MethodChannel _channel = MethodChannel('simdeck_flutter_inspector');
const int _defaultHierarchyMaxDepth = 48;
const int _maxHierarchyDepth = 64;

SimDeckFlutterInspector? _sharedInspector;

SimDeckFlutterInspector startSimDeckFlutterInspector({
  String host = '127.0.0.1',
  String path = '/api/inspector/connect',
  int port = 4310,
  bool reconnect = true,
  bool secure = false,
}) {
  if (_sharedInspector != null) {
    return _sharedInspector!;
  }
  final inspector = SimDeckFlutterInspector(
    SimDeckFlutterInspectorOptions(
      host: host,
      path: path,
      port: port,
      reconnect: reconnect,
      secure: secure,
    ),
  );
  _sharedInspector = inspector;
  inspector.start();
  return inspector;
}

void stopSimDeckFlutterInspector() {
  _sharedInspector?.stop();
  _sharedInspector = null;
}

@immutable
class SimDeckFlutterInspectorOptions {
  const SimDeckFlutterInspectorOptions({
    this.host = '127.0.0.1',
    this.path = '/api/inspector/connect',
    this.port = 4310,
    this.reconnect = true,
    this.secure = false,
  });

  final String host;
  final String path;
  final int port;
  final bool reconnect;
  final bool secure;
}

class SimDeckFlutterInspector {
  SimDeckFlutterInspector([SimDeckFlutterInspectorOptions? options])
      : options = options ?? const SimDeckFlutterInspectorOptions();

  final SimDeckFlutterInspectorOptions options;
  final Expando<String> _ids = Expando<String>('simdeckFlutterInspectorId');
  final Expando<Object> _sourceLocations = Expando<Object>(
    'simdeckFlutterSourceLocation',
  );
  final Map<String, Element> _objects = <String, Element>{};
  final Map<int, _FrameCacheEntry> _frameCache = <int, _FrameCacheEntry>{};
  int _nextObjectId = 1;
  io.WebSocket? _socket;
  Timer? _pollTimer;
  Timer? _reconnectTimer;
  bool _started = false;
  bool _polling = false;
  Map<String, Object?>? _metadata;
  SemanticsHandle? _semanticsHandle;

  void start() {
    stop();
    _started = true;
    _semanticsHandle = SemanticsBinding.instance.ensureSemantics();
    unawaited(_connect());
    _startPolling();
  }

  void stop() {
    _reconnectTimer?.cancel();
    _reconnectTimer = null;
    _pollTimer?.cancel();
    _pollTimer = null;
    _started = false;
    _polling = false;
    _semanticsHandle?.dispose();
    _semanticsHandle = null;
    final socket = _socket;
    _socket = null;
    unawaited(socket?.close());
  }

  Future<void> _connect() async {
    final scheme = options.secure ? 'wss' : 'ws';
    final url = '$scheme://${options.host}:${options.port}${options.path}';
    io.WebSocket? connectedSocket;
    try {
      final socket = await io.WebSocket.connect(url);
      connectedSocket = socket;
      _socket = socket;
      await _sendReady(socket);
      await for (final message in socket) {
        if (message is String) {
          final response = await _executeRequest(_decodeRequest(message));
          socket.add(jsonEncode(response));
        }
      }
    } catch (_) {
      // SimDeck may not be running yet; reconnect below keeps startup cheap.
    } finally {
      if (_started &&
          options.reconnect &&
          identical(_socket, connectedSocket)) {
        _socket = null;
        _scheduleReconnect();
      }
    }
  }

  Future<void> _sendReady(io.WebSocket socket) async {
    socket.add(
      jsonEncode(<String, Object?>{
        'method': 'Inspector.ready',
        'params': await _info(),
      }),
    );
  }

  void _scheduleReconnect() {
    if (_reconnectTimer != null) {
      return;
    }
    _reconnectTimer = Timer(const Duration(seconds: 1), () {
      _reconnectTimer = null;
      unawaited(_connect());
    });
  }

  void _startPolling() {
    if (_polling) {
      return;
    }
    _polling = true;
    _schedulePoll(Duration.zero);
  }

  void _schedulePoll(Duration delay) {
    if (!_polling) {
      return;
    }
    _pollTimer?.cancel();
    _pollTimer = Timer(delay, () {
      _pollTimer = null;
      unawaited(_pollCommands());
    });
  }

  Future<void> _pollCommands() async {
    if (!_polling) {
      return;
    }
    try {
      final info = await _info();
      final pid = info['processIdentifier'];
      final client = io.HttpClient();
      try {
        final request = await client.getUrl(
          Uri.parse('${_httpBaseUrl()}/api/inspector/poll?pid=$pid'),
        );
        final response = await request.close();
        if (response.statusCode == io.HttpStatus.noContent) {
          _schedulePoll(Duration.zero);
          return;
        }
        if (response.statusCode < 200 || response.statusCode >= 300) {
          throw StateError(
            'Inspector poll failed with HTTP ${response.statusCode}.',
          );
        }
        final body = await utf8.decoder.bind(response).join();
        final result = await _executeRequest(_decodeRequest(body));
        final post = await client.postUrl(
          Uri.parse('${_httpBaseUrl()}/api/inspector/response'),
        );
        post.headers.contentType = io.ContentType.json;
        post.add(
          utf8.encode(
            jsonEncode(<String, Object?>{'processIdentifier': pid, ...result}),
          ),
        );
        await post.close();
      } finally {
        client.close(force: true);
      }
      _schedulePoll(Duration.zero);
    } catch (_) {
      _schedulePoll(const Duration(milliseconds: 500));
    }
  }

  String _httpBaseUrl() {
    final scheme = options.secure ? 'https' : 'http';
    return '$scheme://${options.host}:${options.port}';
  }

  Map<String, Object?> _decodeRequest(String data) {
    final decoded = jsonDecode(data);
    if (decoded is Map<String, Object?>) {
      return decoded;
    }
    if (decoded is Map) {
      return decoded.cast<String, Object?>();
    }
    throw const SimDeckFlutterInspectorFailure(
      -32600,
      'Inspector request must be a JSON object.',
    );
  }

  Future<Map<String, Object?>> _executeRequest(
    Map<String, Object?> request,
  ) async {
    try {
      final method = request['method'];
      if (method is! String || method.isEmpty) {
        throw const SimDeckFlutterInspectorFailure(
          -32600,
          'Inspector request requires method.',
        );
      }
      final params = _objectMap(request['params']) ?? <String, Object?>{};
      final result = await _dispatch(method, params);
      return <String, Object?>{'id': request['id'], 'result': result};
    } catch (error) {
      return <String, Object?>{
        'id': request['id'],
        'error': _inspectorError(error),
      };
    }
  }

  Future<Object?> _dispatch(String method, Map<String, Object?> params) async {
    switch (method) {
      case 'Runtime.ping':
        return <String, Object?>{
          'ok': true,
          'protocolVersion': _protocolVersion,
        };
      case 'Inspector.getInfo':
        return _info();
      case 'View.getHierarchy':
        return _hierarchy(params);
      case 'View.get':
        return _getView(params);
      case 'View.hitTest':
        return _hitTest(params);
      case 'View.describeAtPoint':
        return _describeAtPoint(params);
      case 'View.listActions':
        return _listActions(params);
      case 'View.perform':
        return _perform(params);
      case 'View.getProperties':
        return _getProperties(params);
      case 'View.setProperty':
        throw const SimDeckFlutterInspectorFailure(
          -32012,
          'Flutter widgets are immutable; View.setProperty is not supported. Use View.perform for supported runtime actions.',
        );
      default:
        throw SimDeckFlutterInspectorFailure(
          -32601,
          'Unknown inspector method: $method',
        );
    }
  }

  Future<Map<String, Object?>> _info() async {
    final metadata = await _loadMetadata();
    final root = WidgetsBinding.instance.rootElement;
    return <String, Object?>{
      'protocolVersion': _protocolVersion,
      'transport': 'websocket',
      'processIdentifier': metadata['processIdentifier'],
      'bundleIdentifier': metadata['bundleIdentifier'],
      'bundleName': metadata['bundleName'],
      'displayScale': metadata['displayScale'],
      'screenBounds': metadata['screenBounds'],
      'coordinateSpace': 'screen-points',
      'methods': <String>[
        'Runtime.ping',
        'Inspector.getInfo',
        'View.getHierarchy',
        'View.get',
        'View.hitTest',
        'View.describeAtPoint',
        'View.listActions',
        'View.perform',
        'View.getProperties',
        'View.setProperty',
      ],
      'appHierarchy': <String, Object?>{
        'source': 'flutter',
        'available': root != null,
        'publishedAt': DateTime.now().toUtc().toIso8601String(),
      },
      'flutter': <String, Object?>{
        'available': true,
        'widgetCreationTracked':
            WidgetInspectorService.instance.isWidgetCreationTracked(),
      },
      'uikit': <String, Object?>{'available': false, 'propertyEditing': false},
    };
  }

  Future<Map<String, Object?>> _loadMetadata() async {
    if (_metadata != null) {
      return _metadata!;
    }
    final view = _firstFlutterView();
    final logicalSize =
        view == null ? Size.zero : view.physicalSize / view.devicePixelRatio;
    final fallback = <String, Object?>{
      'processIdentifier': io.pid,
      'bundleIdentifier': io.Platform.resolvedExecutable,
      'bundleName': 'Flutter',
      'displayScale': view?.devicePixelRatio ?? 1.0,
      'screenBounds': _rectJson(Offset.zero & logicalSize),
    };
    try {
      final native = await _channel.invokeMapMethod<String, Object?>('getInfo');
      _metadata = <String, Object?>{...fallback, if (native != null) ...native};
    } catch (_) {
      _metadata = fallback;
    }
    return _metadata!;
  }

  Map<String, Object?> _hierarchy(Map<String, Object?> params) {
    if (params['source'] == 'uikit') {
      throw const SimDeckFlutterInspectorFailure(
        -32601,
        'Flutter inspector does not expose a raw UIKit hierarchy.',
      );
    }
    final root = WidgetsBinding.instance.rootElement;
    final roots = <Map<String, Object?>>[];
    if (root != null) {
      final context = _TraversalContext();
      final node = _elementNode(
        root,
        includeHidden: params['includeHidden'] == true,
        maxDepth: _hierarchyMaxDepth(params['maxDepth']),
        depth: 0,
        context: context,
      );
      if (node != null) {
        roots.add(node);
      }
    }
    return <String, Object?>{
      ..._snapshotMetadata(),
      'source': 'flutter',
      'roots': roots,
    };
  }

  Map<String, Object?> _getView(Map<String, Object?> params) {
    final id = _requiredString(params, 'id');
    final element = _objects[id];
    if (element == null) {
      throw SimDeckFlutterInspectorFailure(
        -32004,
        'No view was found for id $id.',
      );
    }
    final node = _elementNode(
      element,
      includeHidden: true,
      maxDepth: _hierarchyMaxDepth(params['maxDepth']),
      depth: 0,
      context: _TraversalContext(),
    );
    if (node == null) {
      throw SimDeckFlutterInspectorFailure(
        -32004,
        'No view was found for id $id.',
      );
    }
    return node;
  }

  Map<String, Object?> _hitTest(Map<String, Object?> params) {
    final point = Offset(
      _requiredNumber(params, 'x'),
      _requiredNumber(params, 'y'),
    );
    final chain = _findChainAtPoint(point);
    return <String, Object?>{
      'x': point.dx,
      'y': point.dy,
      'hit': chain.isEmpty ? null : chain.last,
    };
  }

  Map<String, Object?> _describeAtPoint(Map<String, Object?> params) {
    final point = Offset(
      _requiredNumber(params, 'x'),
      _requiredNumber(params, 'y'),
    );
    final chain = _findChainAtPoint(point);
    return <String, Object?>{
      'x': point.dx,
      'y': point.dy,
      'hit': chain.isEmpty ? null : chain.last,
      'chain': chain,
    };
  }

  Map<String, Object?> _listActions(Map<String, Object?> params) {
    final id = _requiredString(params, 'id');
    final element = _requireElement(id);
    return <String, Object?>{'id': id, 'actions': _actionsFor(element)};
  }

  Future<Map<String, Object?>> _perform(Map<String, Object?> params) async {
    final id = _requiredString(params, 'id');
    final action = _requiredString(params, 'action');
    final element = _requireElement(id);
    switch (action) {
      case 'describe':
        return <String, Object?>{
          'ok': true,
          'id': id,
          'actions': _actionsFor(element),
        };
      case 'tap':
        return _performSemanticsAction(element, action, SemanticsAction.tap);
      case 'longPress':
        return _performSemanticsAction(
          element,
          action,
          SemanticsAction.longPress,
        );
      case 'increase':
        return _performSemanticsAction(
          element,
          action,
          SemanticsAction.increase,
        );
      case 'decrease':
        return _performSemanticsAction(
          element,
          action,
          SemanticsAction.decrease,
        );
      case 'focus':
        FocusScope.of(element).requestFocus(Focus.of(element));
        return <String, Object?>{'ok': true, 'action': action};
      case 'resignFirstResponder':
        FocusScope.of(element).unfocus();
        return <String, Object?>{'ok': true, 'action': action};
      case 'setText':
        _setEditableText(element, _stringValue(params['value']));
        return <String, Object?>{'ok': true, 'action': action};
      case 'scrollBy':
      case 'scrollTo':
        await _scroll(element, params, relative: action == 'scrollBy');
        return <String, Object?>{'ok': true, 'action': action};
      default:
        throw SimDeckFlutterInspectorFailure(
          -32010,
          'Unsupported view action: $action',
        );
    }
  }

  Map<String, Object?> _performSemanticsAction(
    Element element,
    String actionName,
    SemanticsAction action,
  ) {
    final renderObject = element.findRenderObject();
    final semantics = renderObject?.debugSemantics;
    final owner = renderObject?.owner?.semanticsOwner;
    if (semantics == null || owner == null) {
      return <String, Object?>{'ok': false, 'action': actionName};
    }
    final data = semantics.getSemanticsData();
    if (!data.hasAction(action)) {
      return <String, Object?>{'ok': false, 'action': actionName};
    }
    owner.performAction(semantics.id, action);
    return <String, Object?>{'ok': true, 'action': actionName};
  }

  Map<String, Object?> _getProperties(Map<String, Object?> params) {
    final id = _requiredString(params, 'id');
    final element = _requireElement(id);
    final widgetProperties = _diagnosticProperties(element.widget);
    final state = element is StatefulElement ? element.state : null;
    return <String, Object?>{
      'id': id,
      'className': element.widget.runtimeType.toString(),
      'editableProperties': <String>[],
      'properties': widgetProperties,
      'flutter': <String, Object?>{
        'widgetType': element.widget.runtimeType.toString(),
        'elementType': element.runtimeType.toString(),
        'stateType': state?.runtimeType.toString(),
        'key': element.widget.key?.toString(),
        'depth': element.depth,
      },
      'renderObject': _renderObjectProperties(element.findRenderObject()),
      'semantics': _semanticsInfo(element.findRenderObject()),
    };
  }

  Map<String, Object?>? _elementNode(
    Element element, {
    required bool includeHidden,
    required int? maxDepth,
    required int depth,
    required _TraversalContext context,
  }) {
    if (context.expired) {
      return null;
    }
    context.remainingNodes -= 1;

    final hidden = _isHidden(element);
    if (hidden && !includeHidden) {
      return null;
    }

    final type = element.widget.runtimeType.toString();
    final renderObject = element.findRenderObject();
    final frame = _frameFor(renderObject);
    final semantics = _semanticsInfo(renderObject);
    final transparent = _isTransparentWrapper(element, semantics);
    final transparentHitTarget = _isTransparentHitTarget(element, semantics);
    final childDepth = transparent ? depth : depth + 1;
    final children = <Map<String, Object?>>[];
    if (maxDepth == null || transparent || depth < maxDepth) {
      element.visitChildren((child) {
        final node = _elementNode(
          child,
          includeHidden: includeHidden,
          maxDepth: maxDepth,
          depth: childDepth,
          context: context,
        );
        if (node != null) {
          children.add(node);
        }
      });
    }

    if (transparent && children.length == 1) {
      return children.single;
    }

    final sourceLocation = _shouldReadSourceLocation(element, semantics)
        ? _sourceLocation(element)
        : null;
    final id = _objectId(element);
    final title = _nodeTitle(element, semantics);
    return <String, Object?>{
      'id': id,
      'inspectorId': id,
      'type': type,
      'displayName': type,
      'title': title,
      'source': 'flutter',
      'sourceLocation': sourceLocation,
      'sourceLocations': [if (sourceLocation != null) sourceLocation],
      'frame': frame,
      'frameInScreen': frame,
      'AXIdentifier': _accessibilityIdentifier(element, semantics),
      'AXLabel': _firstString(<Object?>[semantics?['label'], title]),
      'AXValue': _firstString(<Object?>[semantics?['value']]),
      'help': _firstString(<Object?>[semantics?['hint']]),
      'enabled': hidden ? false : null,
      'isHidden': hidden,
      'custom_actions': semantics?['actions'],
      'flutter': <String, Object?>{
        'widgetType': type,
        'elementType': element.runtimeType.toString(),
        'stateType': element is StatefulElement
            ? element.state.runtimeType.toString()
            : null,
        'key': element.widget.key?.toString(),
        'depth': element.depth,
        'transparent': transparentHitTarget,
        'compacted': transparent,
      },
      'semantics': semantics,
      'children': children,
    };
  }

  List<Map<String, Object?>> _findChainAtPoint(Offset point) {
    final root = WidgetsBinding.instance.rootElement;
    if (root == null) {
      return <Map<String, Object?>>[];
    }
    final chain = <Map<String, Object?>>[];
    void visit(Element element) {
      final node = _elementNode(
        element,
        includeHidden: false,
        maxDepth: 0,
        depth: 0,
        context: _TraversalContext(),
      );
      final frame =
          _objectMap(node?['frame']) ?? _objectMap(node?['frameInScreen']);
      if (node == null || frame == null || !_frameContains(frame, point)) {
        return;
      }
      final flutter = _objectMap(node['flutter']);
      if (flutter?['transparent'] != true) {
        chain.add(node);
      }
      element.visitChildren(visit);
    }

    visit(root);
    return chain;
  }

  Future<void> _scroll(
    Element element,
    Map<String, Object?> params, {
    required bool relative,
  }) async {
    final scrollable = _findScrollable(element);
    if (scrollable == null) {
      throw const SimDeckFlutterInspectorFailure(
        -32010,
        'Selected Flutter node is not inside a Scrollable.',
      );
    }
    final position = scrollable.position;
    final dx = _optionalDouble(params['x']) ?? 0.0;
    final dy = _optionalDouble(params['y']) ?? 0.0;
    final delta = dy.abs() >= dx.abs() ? dy : dx;
    final target = relative ? position.pixels + delta : delta;
    final clamped = target.clamp(
      position.minScrollExtent,
      position.maxScrollExtent,
    );
    if (params['animated'] == true) {
      await position.animateTo(
        clamped,
        duration: const Duration(milliseconds: 250),
        curve: Curves.easeOut,
      );
    } else {
      position.jumpTo(clamped);
    }
  }

  ScrollableState? _findScrollable(Element element) {
    final direct = Scrollable.maybeOf(element);
    if (direct != null) {
      return direct;
    }
    ScrollableState? result;
    void visit(Element child) {
      result ??= Scrollable.maybeOf(child);
      if (result != null) {
        return;
      }
      child.visitChildren(visit);
    }

    element.visitChildren(visit);
    return result;
  }

  void _setEditableText(Element element, String value) {
    final editable = _findEditableTextState(element);
    if (editable == null) {
      throw const SimDeckFlutterInspectorFailure(
        -32010,
        'Selected Flutter node does not contain EditableText.',
      );
    }
    editable.userUpdateTextEditingValue(
      TextEditingValue(
        text: value,
        selection: TextSelection.collapsed(offset: value.length),
      ),
      SelectionChangedCause.keyboard,
    );
  }

  EditableTextState? _findEditableTextState(Element element) {
    if (element is StatefulElement && element.state is EditableTextState) {
      return element.state as EditableTextState;
    }
    EditableTextState? result;
    void visit(Element child) {
      result ??= _findEditableTextState(child);
    }

    element.visitChildren(visit);
    return result;
  }

  List<String> _actionsFor(Element element) {
    final actions = <String>{'describe', 'getProperties'};
    final semantics = element.findRenderObject()?.debugSemantics;
    if (semantics != null) {
      final data = semantics.getSemanticsData();
      for (final action in <SemanticsAction>[
        SemanticsAction.tap,
        SemanticsAction.longPress,
        SemanticsAction.increase,
        SemanticsAction.decrease,
      ]) {
        if (data.hasAction(action)) {
          actions.add(action.name);
        }
      }
    }
    if (_findEditableTextState(element) != null) {
      actions.add('setText');
      actions.add('focus');
      actions.add('resignFirstResponder');
    }
    if (_findScrollable(element) != null) {
      actions.add('scrollBy');
      actions.add('scrollTo');
    }
    return actions.toList()..sort();
  }

  int? _hierarchyMaxDepth(Object? value) {
    final requested = _optionalInt(value) ?? _defaultHierarchyMaxDepth;
    if (requested < 0) {
      return 0;
    }
    return math.min(requested, _maxHierarchyDepth);
  }

  String _objectId(Element element) {
    final existing = _ids[element];
    if (existing != null) {
      return existing;
    }
    final id = 'flutter:${_nextObjectId++}';
    _ids[element] = id;
    _objects[id] = element;
    return id;
  }

  Element _requireElement(String id) {
    final element = _objects[id];
    if (element == null) {
      throw SimDeckFlutterInspectorFailure(
        -32004,
        'No view was found for id $id.',
      );
    }
    return element;
  }

  Map<String, Object?> _snapshotMetadata() {
    final metadata = _metadata ?? <String, Object?>{};
    return <String, Object?>{
      'capturedAt': DateTime.now().toUtc().toIso8601String(),
      'protocolVersion': _protocolVersion,
      'processIdentifier': metadata['processIdentifier'] ?? io.pid,
      'bundleIdentifier': metadata['bundleIdentifier'],
      'displayScale': metadata['displayScale'] ??
          _firstFlutterView()?.devicePixelRatio ??
          1.0,
      'coordinateSpace': 'screen-points',
    };
  }

  Map<String, Object?>? _frameFor(RenderObject? renderObject) {
    if (renderObject == null || !renderObject.attached) {
      return null;
    }
    final cached = _frameCache[identityHashCode(renderObject)];
    final now = DateTime.now();
    if (cached != null &&
        now.difference(cached.measuredAt).inMilliseconds <= 250) {
      return cached.frame;
    }
    try {
      final rect = MatrixUtils.transformRect(
        renderObject.getTransformTo(null),
        renderObject.paintBounds,
      );
      if (!rect.isFinite || rect.isEmpty) {
        return null;
      }
      final frame = _rectJson(rect);
      _frameCache[identityHashCode(renderObject)] = _FrameCacheEntry(
        frame,
        now,
      );
      return frame;
    } catch (_) {
      return null;
    }
  }

  Map<String, Object?>? _semanticsInfo(RenderObject? renderObject) {
    final node = renderObject?.debugSemantics;
    if (node == null) {
      return null;
    }
    try {
      final data = node.getSemanticsData();
      final actions = <String>[
        for (final action in SemanticsAction.values)
          if (data.hasAction(action)) action.name,
      ];
      return <String, Object?>{
        'id': node.id,
        'identifier': data.identifier,
        'label': data.attributedLabel.string,
        'value': data.attributedValue.string,
        'hint': data.attributedHint.string,
        'tooltip': data.tooltip,
        'role': data.role.name,
        'actions': actions,
        'flags': data.flagsCollection.toString(),
        'isFocused': data.flagsCollection.isFocused.toString(),
        'isTextField': data.flagsCollection.isTextField,
        'isButton': data.flagsCollection.isButton,
        'isEnabled': data.flagsCollection.isEnabled.toString(),
        'isChecked': data.flagsCollection.isChecked.toString(),
        'isSelected': data.flagsCollection.isSelected.toString(),
      };
    } catch (_) {
      return null;
    }
  }

  Map<String, Object?>? _sourceLocation(Element element) {
    if (!WidgetInspectorService.instance.isWidgetCreationTracked()) {
      return null;
    }
    final cached = _sourceLocations[element];
    if (cached != null) {
      return identical(cached, _noSourceLocation)
          ? null
          : cached as Map<String, Object?>;
    }
    try {
      final json = element.toDiagnosticsNode().toJsonMap(
            InspectorSerializationDelegate(
              service: WidgetInspectorService.instance,
              subtreeDepth: 0,
            ),
          );
      final location = _objectMap(json['creationLocation']);
      if (location == null) {
        _sourceLocations[element] = _noSourceLocation;
        return null;
      }
      final sourceLocation = <String, Object?>{
        'file': location['file'],
        'line': location['line'],
        'column': location['column'],
        'name': location['name'],
        'kind': 'flutter',
      };
      _sourceLocations[element] = sourceLocation;
      return sourceLocation;
    } catch (_) {
      _sourceLocations[element] = _noSourceLocation;
      return null;
    }
  }

  bool _shouldReadSourceLocation(
    Element element,
    Map<String, Object?>? semantics,
  ) {
    if (!WidgetInspectorService.instance.isWidgetCreationTracked()) {
      return false;
    }
    final type = element.widget.runtimeType.toString();
    return element.widget.key != null ||
        _hasSemanticContent(semantics) ||
        !_isFrameworkWrapperType(type);
  }

  String _nodeTitle(Element element, Map<String, Object?>? semantics) {
    final widget = element.widget;
    final type = widget.runtimeType.toString();
    final semanticTitle = _firstString(<Object?>[semantics?['label']]);
    if (semanticTitle != null) {
      return semanticTitle;
    }
    if (_isTransparentContainerType(type)) {
      return _firstString(<Object?>[widget.key?.toString(), type]) ?? '';
    }
    return _firstString(<Object?>[
          _diagnosticTitle(widget),
          widget.key?.toString(),
          type,
        ]) ??
        '';
  }

  String? _accessibilityIdentifier(
    Element element,
    Map<String, Object?>? semantics,
  ) {
    return _firstString(<Object?>[
      semantics?['identifier'],
      element.widget.key?.toString(),
    ]);
  }

  bool _isHidden(Element element) {
    final widget = element.widget;
    if (widget is Offstage && widget.offstage) {
      return true;
    }
    if (widget is Visibility && !widget.visible) {
      return true;
    }
    if (widget is Opacity && widget.opacity <= 0.0) {
      return true;
    }
    final frame = _frameFor(element.findRenderObject());
    if (frame != null) {
      final width = _optionalDouble(frame['width']) ?? 0.0;
      final height = _optionalDouble(frame['height']) ?? 0.0;
      if (width <= 0.0 || height <= 0.0) {
        return true;
      }
    }
    return false;
  }

  bool _isTransparentWrapper(Element element, Map<String, Object?>? semantics) {
    final type = element.widget.runtimeType.toString();
    if (_isTransparentContainerType(type)) {
      return !_isSemanticContentWidgetType(type, semantics);
    }
    if (_hasSemanticContent(semantics)) {
      return false;
    }
    if (element.widget.key != null) {
      return false;
    }
    return false;
  }

  bool _isTransparentHitTarget(
    Element element,
    Map<String, Object?>? semantics,
  ) {
    final type = element.widget.runtimeType.toString();
    if (_isTransparentContainerType(type)) {
      return !_isSemanticContentWidgetType(type, semantics);
    }
    return !_hasSemanticContent(semantics) && _isTransparentContainerType(type);
  }

  bool _isTransparentContainerType(String type) {
    return _isFrameworkWrapperType(type) ||
        _isFlutterPassThroughWidgetType(type);
  }

  bool _isSemanticContentWidgetType(
    String type,
    Map<String, Object?>? semantics,
  ) {
    final baseType = _baseWidgetType(type);
    if (baseType == 'Semantics') {
      return _hasSemanticContent(semantics);
    }
    return baseType == 'Text' ||
        baseType == 'RichText' ||
        baseType == 'EditableText';
  }

  Map<String, Object?> _diagnosticProperties(Object object) {
    if (object is! Diagnosticable) {
      return <String, Object?>{};
    }
    final properties = <String, Object?>{};
    for (final property in object.toDiagnosticsNode().getProperties()) {
      final name = property.name;
      if (name == null || name.isEmpty) {
        continue;
      }
      properties[name] = _encodeDiagnosticsValue(
        property.value,
        property.toDescription(),
      );
    }
    return properties;
  }

  String? _diagnosticTitle(Object object) {
    if (object is! Diagnosticable) {
      return null;
    }
    final properties = <String, Object?>{};
    for (final property in object.toDiagnosticsNode().getProperties()) {
      final name = property.name;
      if (name == null || !_titleDiagnosticPropertyNames.contains(name)) {
        continue;
      }
      properties[name] = _encodeDiagnosticsValue(
        property.value,
        property.toDescription(),
      );
    }
    return _firstString(<Object?>[
      properties['label'],
      properties['text'],
      properties['data'],
      properties['message'],
      properties['tooltip'],
      properties['semanticLabel'],
      properties['value'],
    ]);
  }

  Map<String, Object?>? _renderObjectProperties(RenderObject? renderObject) {
    if (renderObject == null) {
      return null;
    }
    return <String, Object?>{
      'type': renderObject.runtimeType.toString(),
      'attached': renderObject.attached,
      'needsLayout': renderObject.debugNeedsLayout,
      'needsPaint': renderObject.debugNeedsPaint,
      'paintBounds': _rectJson(renderObject.paintBounds),
    };
  }
}

class SimDeckFlutterInspectorFailure implements Exception {
  const SimDeckFlutterInspectorFailure(this.code, this.message);

  final int code;
  final String message;

  @override
  String toString() => message;
}

class _TraversalContext {
  _TraversalContext()
      : deadline = DateTime.now().add(const Duration(seconds: 3));

  final DateTime deadline;
  int remainingNodes = 3500;

  bool get expired => remainingNodes <= 0 || DateTime.now().isAfter(deadline);
}

class _FrameCacheEntry {
  const _FrameCacheEntry(this.frame, this.measuredAt);

  final Map<String, Object?> frame;
  final DateTime measuredAt;
}

final Object _noSourceLocation = Object();

const Set<String> _titleDiagnosticPropertyNames = <String>{
  'label',
  'text',
  'data',
  'message',
  'tooltip',
  'semanticLabel',
  'value',
};

const Set<String> _frameworkWrapperTypes = <String>{
  'Actions',
  '_ActionsScope',
  'AnimatedTheme',
  'Builder',
  'CheckedModeBanner',
  'CupertinoPageTransition',
  'CupertinoTheme',
  'DecoratedBoxTransition',
  'DefaultSelectionStyle',
  'DefaultTextEditingShortcuts',
  'Directionality',
  'Focus',
  '_FocusInheritedScope',
  '_FocusScopeWithExternalFocusNode',
  'FocusTraversalGroup',
  'HeroControllerScope',
  'IconTheme',
  'InheritedCupertinoTheme',
  'Localizations',
  '_LocalizationsScope',
  'MaterialApp',
  'MediaQuery',
  '_MediaQueryFromView',
  'ModalBarrier',
  'Navigator',
  'NotificationListener<NavigationNotification>',
  'Overlay',
  'PageStorage',
  'RawView',
  '_RawViewInternal',
  'RootWidget',
  'ScaffoldMessenger',
  'ScrollConfiguration',
  'ShortcutRegistrar',
  'Semantics',
  'Shortcuts',
  '_ShortcutsMarker',
  'SlideTransition',
  'Theme',
  'Title',
  'View',
  '_ViewScope',
  'WidgetsApp',
  '_WidgetsAppState',
  '_PipelineOwnerScope',
  '_InheritedTheme',
};

const Set<String> _flutterPassThroughWidgetTypes = <String>{
  'AbsorbPointer',
  'Align',
  'AnimatedBuilder',
  'AnimatedDefaultTextStyle',
  'AnimatedOpacity',
  'AnimatedPadding',
  'AnimatedPhysicalModel',
  'AnimatedPositioned',
  'AnimatedContainer',
  'AspectRatio',
  'AutomaticKeepAlive',
  'BlockSemantics',
  'Center',
  'ClipPath',
  'ClipRRect',
  'ClipRect',
  'Column',
  'CompositedTransformFollower',
  'CompositedTransformTarget',
  'ConstrainedBox',
  'Container',
  'CustomMultiChildLayout',
  'CustomPaint',
  'CustomSingleChildLayout',
  'DecoratedBox',
  'DefaultTextStyle',
  'ExcludeSemantics',
  'Expanded',
  'Flexible',
  'FocusScope',
  'FractionalTranslation',
  'GestureDetector',
  'IgnorePointer',
  'IconButtonTheme',
  'Ink',
  'IndexedSemantics',
  'InputDecorator',
  'IntrinsicHeight',
  'IntrinsicWidth',
  'KeepAlive',
  'KeyedSubtree',
  'LayoutId',
  'LayoutBuilder',
  'LimitedBox',
  'Listener',
  'ListenableBuilder',
  'ListView',
  'Material',
  'MatrixTransition',
  'MouseRegion',
  'NotificationListener',
  'Offstage',
  'Opacity',
  'OverflowBox',
  'Padding',
  'PhysicalModel',
  'PhysicalShape',
  'Positioned',
  'PositionedDirectional',
  'PrimaryScrollController',
  'RawGestureDetector',
  'RepaintBoundary',
  'RestorationScope',
  'RootRestorationScope',
  'Row',
  'SafeArea',
  'Scaffold',
  'ScrollNotificationObserver',
  'Scrollable',
  'SharedAppData',
  'SizeChangedLayoutNotifier',
  'SizedBox',
  'SliverList',
  'SliverPadding',
  'Stack',
  'TapRegionSurface',
  'TextFieldTapRegion',
  'TextSelectionGestureDetector',
  'TickerMode',
  'Transform',
  'UndoHistory',
  'UnmanagedRestorationScope',
  'ValueListenableBuilder',
  'Viewport',
};

bool _isFrameworkWrapperType(String type) {
  final baseType = _baseWidgetType(type);
  return type.startsWith('_') ||
      baseType.startsWith('_') ||
      _frameworkWrapperTypes.contains(type) ||
      _frameworkWrapperTypes.contains(baseType);
}

bool _isFlutterPassThroughWidgetType(String type) {
  final baseType = _baseWidgetType(type);
  return _flutterPassThroughWidgetTypes.contains(type) ||
      _flutterPassThroughWidgetTypes.contains(baseType);
}

String _baseWidgetType(String type) {
  final genericStart = type.indexOf('<');
  return genericStart < 0 ? type : type.substring(0, genericStart);
}

bool _hasSemanticContent(Map<String, Object?>? semantics) {
  if (semantics == null) {
    return false;
  }
  final label = _firstString(<Object?>[
    semantics['identifier'],
    semantics['label'],
    semantics['value'],
    semantics['hint'],
    semantics['tooltip'],
  ]);
  final actions = semantics['actions'];
  return label != null || (actions is List && actions.isNotEmpty);
}

Map<String, Object?> _inspectorError(Object error) {
  if (error is SimDeckFlutterInspectorFailure) {
    return <String, Object?>{'code': error.code, 'message': error.message};
  }
  return <String, Object?>{'code': -32011, 'message': error.toString()};
}

Map<String, Object?>? _objectMap(Object? value) {
  if (value is Map<String, Object?>) {
    return value;
  }
  if (value is Map) {
    return value.cast<String, Object?>();
  }
  return null;
}

String _requiredString(Map<String, Object?> params, String key) {
  final value = params[key];
  if (value is String && value.isNotEmpty) {
    return value;
  }
  throw SimDeckFlutterInspectorFailure(
    -32600,
    'Request params.$key must be a non-empty string.',
  );
}

double _requiredNumber(Map<String, Object?> params, String key) {
  final value = _optionalDouble(params[key]);
  if (value != null && value.isFinite) {
    return value;
  }
  throw SimDeckFlutterInspectorFailure(
    -32600,
    'Request params.$key must be a finite number.',
  );
}

int? _optionalInt(Object? value) {
  if (value is int) {
    return value;
  }
  if (value is num && value.isFinite) {
    return value.toInt();
  }
  return null;
}

double? _optionalDouble(Object? value) {
  if (value is num && value.isFinite) {
    return value.toDouble();
  }
  return null;
}

String _stringValue(Object? value) => value == null ? '' : value.toString();

String? _firstString(Iterable<Object?> values) {
  for (final value in values) {
    final text = value?.toString().trim();
    if (text != null && text.isNotEmpty && text != 'null') {
      return text;
    }
  }
  return null;
}

Object? _encodeDiagnosticsValue(Object? value, String description) {
  if (value == null || value is num || value is bool || value is String) {
    return value;
  }
  if (value is Color) {
    final argb = value.toARGB32();
    return <String, Object?>{
      r'$type': 'Color',
      'value': argb,
      'hex': '#${argb.toRadixString(16).padLeft(8, '0').toUpperCase()}',
    };
  }
  if (value is EdgeInsetsGeometry ||
      value is AlignmentGeometry ||
      value is TextStyle ||
      value is BoxDecoration) {
    return description;
  }
  return description.isEmpty ? value.toString() : description;
}

Map<String, Object?> _rectJson(Rect rect) {
  return <String, Object?>{
    'x': _finiteOrZero(rect.left),
    'y': _finiteOrZero(rect.top),
    'width': math.max(0.0, _finiteOrZero(rect.width)),
    'height': math.max(0.0, _finiteOrZero(rect.height)),
  };
}

bool _frameContains(Map<String, Object?> frame, Offset point) {
  final x = _optionalDouble(frame['x']);
  final y = _optionalDouble(frame['y']);
  final width = _optionalDouble(frame['width']);
  final height = _optionalDouble(frame['height']);
  if (x == null || y == null || width == null || height == null) {
    return false;
  }
  return point.dx >= x &&
      point.dy >= y &&
      point.dx <= x + width &&
      point.dy <= y + height;
}

double _finiteOrZero(double value) => value.isFinite ? value : 0.0;

FlutterView? _firstFlutterView() {
  final views = WidgetsBinding.instance.platformDispatcher.views;
  return views.isEmpty ? null : views.first;
}
