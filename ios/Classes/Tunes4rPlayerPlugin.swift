import Flutter
import Foundation

public class Tunes4rPlayerPlugin: NSObject, FlutterPlugin {
  public static func register(with registrar: FlutterPluginRegistrar) {
    // FFI plugin - native functions are accessed via DynamicLibrary.process()
    // No Flutter method channels needed; all communication is through Dart FFI.
    let channel = FlutterMethodChannel(
      name: "tunes4r_player",
      binaryMessenger: registrar.messenger()
    )
    let instance = Tunes4rPlayerPlugin()
    registrar.addMethodCallDelegate(instance, channel: channel)
  }

  public func handle(_ call: FlutterMethodCall, result: @escaping FlutterResult) {
    result(FlutterMethodNotImplemented)
  }
}
