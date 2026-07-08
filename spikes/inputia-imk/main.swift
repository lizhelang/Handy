import Cocoa
import InputMethodKit

@main
struct InputiaSpikeApp {
  private static var server: IMKServer?

  static func main() {
    autoreleasepool {
      if InputiaSpikeInstaller.handle(arguments: CommandLine.arguments) {
        return
      }

      let bundle = Bundle.main
      let connectionName = bundle.object(forInfoDictionaryKey: "InputMethodConnectionName") as? String
        ?? "InputiaIMKSpike_Connection"
      let bundleIdentifier = bundle.bundleIdentifier ?? "dev.inputia.inputmethod.Spike"

      server = IMKServer(name: connectionName, bundleIdentifier: bundleIdentifier)

      let app = NSApplication.shared
      let delegate = InputiaSpikeApplicationDelegate()
      app.delegate = delegate
      app.setActivationPolicy(.accessory)
      app.run()
    }
  }
}

final class InputiaSpikeApplicationDelegate: NSObject, NSApplicationDelegate {}

final class InputiaSpikeInstaller {
  private static let inputSourceID = "dev.inputia.inputmethod.Spike"

  static func handle(arguments: [String]) -> Bool {
    guard let command = arguments.dropFirst().first else {
      return false
    }

    let installer = InputiaSpikeInstaller()
    switch command {
    case "--register-input-source", "--install":
      installer.register()
      return true
    case "--enable-input-source":
      installer.enable()
      return true
    case "--disable-input-source":
      installer.disable()
      return true
    case "--select-input-source":
      installer.select(sourceID: arguments.dropFirst(2).first ?? Self.inputSourceID)
      return true
    case "--current-input-source":
      installer.current()
      return true
    case "--dump-inputia-sources":
      installer.dumpInputiaSources()
      return true
    case "--dump-enabled-inputia-sources":
      installer.dumpInputiaSources(includeAllInstalled: false)
      return true
    case "--dump-sources":
      installer.dumpSources(matching: arguments.dropFirst(2).first ?? "")
      return true
    case "--self-check-classes":
      installer.selfCheckClasses()
      return true
    case "--list-input-source":
      installer.list()
      return true
    default:
      return false
    }
  }

  private func register() {
    TISRegisterInputSource(Bundle.main.bundleURL as CFURL)
    print("Registered input source from \(Bundle.main.bundleURL.path)")
  }

  private func enable() {
    enable(sourceID: Self.inputSourceID)
  }

  private func enable(sourceID: String) {
    guard let source = inputSource(sourceID: sourceID) else {
      print("Input source not found: \(sourceID)")
      return
    }

    let error = TISEnableInputSource(source)
    print("Enable \(error == noErr ? "succeeds" : "fails with \(error)") for \(sourceID)")
  }

  private func disable() {
    disable(sourceID: Self.inputSourceID)
  }

  private func disable(sourceID: String) {
    guard let source = inputSource(sourceID: sourceID) else {
      print("Input source not found: \(sourceID)")
      return
    }

    let error = TISDisableInputSource(source)
    print("Disable \(error == noErr ? "succeeds" : "fails with \(error)") for \(sourceID)")
  }

  private func select(sourceID: String) {
    let lookup = inputSourceForSelection(sourceID: sourceID)
    guard let source = lookup.source else {
      print("Input source not found: \(sourceID)")
      return
    }

    print("Selecting from \(lookup.includeAllInstalled ? "all installed" : "enabled") source list")
    printSourceDetails(source)
    print("enabled=\(boolProperty(source, key: kTISPropertyInputSourceIsEnabled).map(String.init) ?? "unknown")")
    print("selectable=\(boolProperty(source, key: kTISPropertyInputSourceIsSelectCapable).map(String.init) ?? "unknown")")
    print("selected=\(boolProperty(source, key: kTISPropertyInputSourceIsSelected).map(String.init) ?? "unknown")")
    let error = TISSelectInputSource(source)
    print("Select \(error == noErr ? "succeeds" : "fails with \(error)") for \(sourceID)")
  }

  private func current() {
    let source = TISCopyCurrentKeyboardInputSource().takeRetainedValue()
    let sourceIDRef = TISGetInputSourceProperty(source, kTISPropertyInputSourceID)
    if let sourceID = unsafeBitCast(sourceIDRef, to: CFString?.self) as String? {
      print(sourceID)
    } else {
      print("unknown")
    }
  }

  private func list() {
    guard let source = inputSource() else {
      print("Input source not found: \(Self.inputSourceID)")
      return
    }

    let enabled = boolProperty(source, key: kTISPropertyInputSourceIsEnabled)
    let selectable = boolProperty(source, key: kTISPropertyInputSourceIsSelectCapable)
    let selected = boolProperty(source, key: kTISPropertyInputSourceIsSelected)
    print("Input source: \(Self.inputSourceID)")
    printSourceDetails(source)
    print("enabled=\(enabled.map(String.init) ?? "unknown")")
    print("selectable=\(selectable.map(String.init) ?? "unknown")")
    print("selected=\(selected.map(String.init) ?? "unknown")")
  }

  private func dumpInputiaSources() {
    dumpInputiaSources(includeAllInstalled: true)
  }

  private func dumpInputiaSources(includeAllInstalled: Bool) {
    let sourceList = TISCreateInputSourceList(nil, includeAllInstalled).takeRetainedValue() as! [TISInputSource]
    var matches = 0
    for source in sourceList {
      let sourceID = stringProperty(source, key: kTISPropertyInputSourceID) ?? ""
      let bundleID = stringProperty(source, key: kTISPropertyBundleID) ?? ""
      if sourceID.localizedCaseInsensitiveContains("inputia")
        || bundleID.localizedCaseInsensitiveContains("inputia") {
        matches += 1
        print("----")
        printSourceDetails(source)
        print("enabled=\(boolProperty(source, key: kTISPropertyInputSourceIsEnabled).map(String.init) ?? "unknown")")
        print("enableCapable=\(boolProperty(source, key: kTISPropertyInputSourceIsEnableCapable).map(String.init) ?? "unknown")")
        print("selectable=\(boolProperty(source, key: kTISPropertyInputSourceIsSelectCapable).map(String.init) ?? "unknown")")
        print("selected=\(boolProperty(source, key: kTISPropertyInputSourceIsSelected).map(String.init) ?? "unknown")")
      }
    }
    print("matches=\(matches)")
  }

  private func dumpSources(matching query: String) {
    let normalizedQuery = query.lowercased()
    for includeAllInstalled in [false, true] {
      print("== includeAllInstalled=\(includeAllInstalled) ==")
      let sourceList = TISCreateInputSourceList(nil, includeAllInstalled).takeRetainedValue() as! [TISInputSource]
      var matches = 0
      for source in sourceList {
        let values = [
          stringProperty(source, key: kTISPropertyInputSourceID),
          stringProperty(source, key: kTISPropertyBundleID),
          stringProperty(source, key: kTISPropertyInputModeID),
          stringProperty(source, key: kTISPropertyLocalizedName),
        ].compactMap { $0 }
        if normalizedQuery.isEmpty || values.contains(where: { $0.lowercased().contains(normalizedQuery) }) {
          matches += 1
          print("----")
          printSourceDetails(source)
          print("enabled=\(boolProperty(source, key: kTISPropertyInputSourceIsEnabled).map(String.init) ?? "unknown")")
          print("enableCapable=\(boolProperty(source, key: kTISPropertyInputSourceIsEnableCapable).map(String.init) ?? "unknown")")
          print("selectable=\(boolProperty(source, key: kTISPropertyInputSourceIsSelectCapable).map(String.init) ?? "unknown")")
          print("selected=\(boolProperty(source, key: kTISPropertyInputSourceIsSelected).map(String.init) ?? "unknown")")
        }
      }
      print("matches=\(matches)")
    }
  }

  private func selfCheckClasses() {
    let bundle = Bundle.main
    for key in ["InputMethodServerControllerClass", "InputMethodServerDelegateClass"] {
      let className = bundle.object(forInfoDictionaryKey: key) as? String ?? ""
      print("\(key)=\(className)")
      print("classFound=\(NSClassFromString(className) != nil)")
    }
  }

  private func inputSourceForSelection(sourceID: String) -> (source: TISInputSource?, includeAllInstalled: Bool) {
    if let source = inputSource(sourceID: sourceID, includeAllInstalled: false) {
      return (source, false)
    }
    return (inputSource(sourceID: sourceID, includeAllInstalled: true), true)
  }

  private func inputSource() -> TISInputSource? {
    inputSource(sourceID: Self.inputSourceID)
  }

  private func inputSource(sourceID expectedSourceID: String) -> TISInputSource? {
    inputSource(sourceID: expectedSourceID, includeAllInstalled: true)
  }

  private func inputSource(sourceID expectedSourceID: String, includeAllInstalled: Bool) -> TISInputSource? {
    let properties = NSMutableDictionary()
    properties.setValue(expectedSourceID, forKey: kTISPropertyInputSourceID as String)
    guard let unmanagedSourceList = TISCreateInputSourceList(properties, includeAllInstalled) else {
      return nil
    }
    let sourceList = unmanagedSourceList.takeRetainedValue() as! [TISInputSource]
    for source in sourceList {
      let sourceIDRef = TISGetInputSourceProperty(source, kTISPropertyInputSourceID)
      guard let sourceID = unsafeBitCast(sourceIDRef, to: CFString?.self) as String? else {
        continue
      }
      if sourceID == expectedSourceID {
        return source
      }
    }
    return nil
  }

  private func printSourceDetails(_ source: TISInputSource) {
    print("id=\(stringProperty(source, key: kTISPropertyInputSourceID) ?? "unknown")")
    print("bundle=\(stringProperty(source, key: kTISPropertyBundleID) ?? "unknown")")
    print("mode=\(stringProperty(source, key: kTISPropertyInputModeID) ?? "unknown")")
    print("name=\(stringProperty(source, key: kTISPropertyLocalizedName) ?? "unknown")")
    print("category=\(stringProperty(source, key: kTISPropertyInputSourceCategory) ?? "unknown")")
    print("type=\(stringProperty(source, key: kTISPropertyInputSourceType) ?? "unknown")")
  }

  private func stringProperty(_ source: TISInputSource, key: CFString!) -> String? {
    let propertyRef = TISGetInputSourceProperty(source, key)
    return unsafeBitCast(propertyRef, to: CFString?.self) as String?
  }

  private func boolProperty(_ source: TISInputSource, key: CFString!) -> Bool? {
    let propertyRef = TISGetInputSourceProperty(source, key)
    guard let value = unsafeBitCast(propertyRef, to: CFBoolean?.self) else {
      return nil
    }
    return CFBooleanGetValue(value)
  }
}

final class InputiaSpikeInputController: IMKInputController {
  private weak var inputClient: IMKTextInput?

  override init!(server: IMKServer!, delegate: Any!, client: Any!) {
    self.inputClient = client as? IMKTextInput
    super.init(server: server, delegate: delegate, client: client)
  }

  override func recognizedEvents(_ sender: Any!) -> Int {
    Int(NSEvent.EventTypeMask.Element(arrayLiteral: .keyDown, .flagsChanged).rawValue)
  }

  override func handle(_ event: NSEvent!, client sender: Any!) -> Bool {
    guard let event else {
      return false
    }

    if event.type == .flagsChanged {
      return false
    }

    guard event.type == .keyDown else {
      return false
    }

    if event.modifierFlags.contains(.command) {
      return false
    }

    guard let text = event.characters, !text.isEmpty else {
      return false
    }

    inputClient = sender as? IMKTextInput
    inputClient?.insertText(text, replacementRange: NSRange(location: NSNotFound, length: 0))
    return true
  }

  override func commitComposition(_ sender: Any!) {
    inputClient = sender as? IMKTextInput
  }
}
