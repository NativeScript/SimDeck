import Flutter
import UIKit

public class SimDeckFlutterInspectorPlugin: NSObject, FlutterPlugin {
  public static func register(with registrar: FlutterPluginRegistrar) {
    let channel = FlutterMethodChannel(
      name: "simdeck_flutter_inspector",
      binaryMessenger: registrar.messenger()
    )
    registrar.addMethodCallDelegate(SimDeckFlutterInspectorPlugin(), channel: channel)
  }

  public func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    switch call.method {
    case "getInfo":
      let bundle = Bundle.main
      let screen = UIScreen.main
      result([
        "processIdentifier": ProcessInfo.processInfo.processIdentifier,
        "bundleIdentifier": bundle.bundleIdentifier as Any,
        "bundleName": bundle.object(forInfoDictionaryKey: "CFBundleName") as? String
          ?? bundle.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String
          ?? "",
        "displayScale": screen.scale,
        "screenBounds": [
          "x": screen.bounds.origin.x,
          "y": screen.bounds.origin.y,
          "width": screen.bounds.size.width,
          "height": screen.bounds.size.height,
        ],
        "systemName": UIDevice.current.systemName,
        "systemVersion": UIDevice.current.systemVersion,
      ])
    default:
      result(FlutterMethodNotImplemented)
    }
  }
}
