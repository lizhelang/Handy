import Carbon
import Foundation

private let defaultAppPath = ProcessInfo.processInfo.environment["INPUTIA_APP"]
  ?? "/Library/Input Methods/InputiaInputMethod.app"
private let bundleIdentifier = ProcessInfo.processInfo.environment["INPUTIA_TIS_BUNDLE_ID"]
  ?? "com.inputia.inputmethod.Inputia"
private let primaryModeIdentifier = ProcessInfo.processInfo.environment["INPUTIA_TIS_MODE_ID"]
  ?? "com.inputia.inputmethod.Inputia.Hans"
private let requiresAppMatch = ProcessInfo.processInfo.environment["INPUTIA_TIS_REQUIRE_APP_MATCH"] == "1"
private let expectedIconPath = URL(fileURLWithPath: defaultAppPath)
  .appendingPathComponent("Contents/Resources/inputia.pdf")
  .standardizedFileURL
  .path

private func stringProperty(_ source: TISInputSource, key: CFString) -> String {
  guard let value = TISGetInputSourceProperty(source, key) else {
    return "unknown"
  }
  return Unmanaged<CFString>.fromOpaque(value).takeUnretainedValue() as String
}

private func boolProperty(_ source: TISInputSource, key: CFString) -> Bool {
  guard let value = TISGetInputSourceProperty(source, key) else {
    return false
  }
  return CFBooleanGetValue(Unmanaged<CFBoolean>.fromOpaque(value).takeUnretainedValue())
}

private func urlProperty(_ source: TISInputSource, key: CFString) -> String {
  guard let value = TISGetInputSourceProperty(source, key) else {
    return "unknown"
  }
  let object = Unmanaged<AnyObject>.fromOpaque(value).takeUnretainedValue()
  guard let url = object as? URL else {
    return "unknown"
  }
  return url.path
}

private func stringArrayProperty(_ source: TISInputSource, key: CFString) -> String {
  guard let value = TISGetInputSourceProperty(source, key) else {
    return "unknown"
  }
  let array = Unmanaged<CFArray>.fromOpaque(value).takeUnretainedValue() as [AnyObject]
  return array.compactMap { $0 as? String }.joined(separator: ",")
}

private func sourceList(includeAllInstalled: Bool) -> [TISInputSource] {
  guard let unmanaged = TISCreateInputSourceList(nil, includeAllInstalled) else {
    return []
  }
  return unmanaged.takeRetainedValue() as! [TISInputSource]
}

private func source(inputSourceID: String, includeAllInstalled: Bool) -> TISInputSource? {
  let properties = NSMutableDictionary()
  properties.setValue(inputSourceID, forKey: kTISPropertyInputSourceID as String)
  guard let unmanaged = TISCreateInputSourceList(properties, includeAllInstalled) else {
    return nil
  }
  let sources = unmanaged.takeRetainedValue() as! [TISInputSource]
  let matchingID = sources.filter {
    stringProperty($0, key: kTISPropertyInputSourceID) == inputSourceID
  }
  return matchingID.first {
    urlProperty($0, key: kTISPropertyIconImageURL) == expectedIconPath
  } ?? (requiresAppMatch ? nil : matchingID.first)
}

private func sourceByID(inputSourceID: String, includeAllInstalled: Bool) -> TISInputSource? {
  let properties = NSMutableDictionary()
  properties.setValue(inputSourceID, forKey: kTISPropertyInputSourceID as String)
  guard let unmanaged = TISCreateInputSourceList(properties, includeAllInstalled) else {
    return nil
  }
  let sources = unmanaged.takeRetainedValue() as! [TISInputSource]
  return sources.first {
    stringProperty($0, key: kTISPropertyInputSourceID) == inputSourceID
  }
}

private func matchingSources(includeAllInstalled: Bool) -> [TISInputSource] {
  sourceList(includeAllInstalled: includeAllInstalled).filter { source in
    let sourceID = stringProperty(source, key: kTISPropertyInputSourceID)
    let bundleID = stringProperty(source, key: kTISPropertyBundleID)
    let modeID = stringProperty(source, key: kTISPropertyInputModeID)
    return sourceID.hasPrefix(bundleIdentifier)
      || bundleID.hasPrefix(bundleIdentifier)
      || modeID.hasPrefix(bundleIdentifier)
  }
}

private func printSource(_ source: TISInputSource) {
  print("id=\(stringProperty(source, key: kTISPropertyInputSourceID))")
  print("bundle=\(stringProperty(source, key: kTISPropertyBundleID))")
  print("mode=\(stringProperty(source, key: kTISPropertyInputModeID))")
  print("name=\(stringProperty(source, key: kTISPropertyLocalizedName))")
  print("category=\(stringProperty(source, key: kTISPropertyInputSourceCategory))")
  print("type=\(stringProperty(source, key: kTISPropertyInputSourceType))")
  print("iconURL=\(urlProperty(source, key: kTISPropertyIconImageURL))")
  print("languages=\(stringArrayProperty(source, key: kTISPropertyInputSourceLanguages))")
  print("enabled=\(boolProperty(source, key: kTISPropertyInputSourceIsEnabled))")
  print("enableCapable=\(boolProperty(source, key: kTISPropertyInputSourceIsEnableCapable))")
  print("selectable=\(boolProperty(source, key: kTISPropertyInputSourceIsSelectCapable))")
  print("selected=\(boolProperty(source, key: kTISPropertyInputSourceIsSelected))")
}

private func dump() {
  for includeAllInstalled in [false, true] {
    print("includeAllInstalled=\(includeAllInstalled)")
    let matches = matchingSources(includeAllInstalled: includeAllInstalled)
    print("matches=\(matches.count)")
    for source in matches {
      printSource(source)
    }
  }
}

private func dumpCurrentInputSource() {
  let currentSource = TISCopyCurrentKeyboardInputSource().takeRetainedValue()
  printSource(currentSource)
}

private func register(appPath: String) {
  let status = TISRegisterInputSource(URL(fileURLWithPath: appPath) as CFURL)
  print("registerStatus=\(status)")
}

private func enable(inputSourceID: String, label: String) {
  guard let inputSource = source(inputSourceID: inputSourceID, includeAllInstalled: true) else {
    print("\(label)Found=false")
    return
  }
  let status = TISEnableInputSource(inputSource)
  print("\(label)EnableStatus=\(status)")
  printSource(inputSource)
}

private func select(inputSourceID: String) {
  guard let inputSource = source(inputSourceID: inputSourceID, includeAllInstalled: false) else {
    print("selectSourceFoundInEnabledList=false")
    return
  }
  let status = TISSelectInputSource(inputSource)
  print("selectStatus=\(status)")
  printSource(inputSource)
  let currentSource = TISCopyCurrentKeyboardInputSource().takeRetainedValue()
  let currentID = stringProperty(currentSource, key: kTISPropertyInputSourceID)
  print("selectExpectedID=\(inputSourceID)")
  print("selectCurrentID=\(currentID)")
  print("selectCurrentMatchesTarget=\(currentID == inputSourceID)")
}

private func selectSourceByID(inputSourceID: String) {
  guard let inputSource = sourceByID(inputSourceID: inputSourceID, includeAllInstalled: false) else {
    print("selectSourceFoundInEnabledList=false")
    print("selectExpectedID=\(inputSourceID)")
    return
  }
  let status = TISSelectInputSource(inputSource)
  print("selectStatus=\(status)")
  printSource(inputSource)
  let currentSource = TISCopyCurrentKeyboardInputSource().takeRetainedValue()
  let currentID = stringProperty(currentSource, key: kTISPropertyInputSourceID)
  print("selectExpectedID=\(inputSourceID)")
  print("selectCurrentID=\(currentID)")
  print("selectCurrentMatchesTarget=\(currentID == inputSourceID)")
}

private func disable(inputSourceID: String, label: String) {
  guard let inputSource = source(inputSourceID: inputSourceID, includeAllInstalled: true) else {
    print("\(label)Found=false")
    return
  }
  let status = TISDisableInputSource(inputSource)
  print("\(label)DisableStatus=\(status)")
  printSource(inputSource)
}

private func printUsage() {
  print("usage=inputia-tis-tool [--dump|--dump-current-input-source|--reset-enable|--select-inputia-source-id <id>|--select-source-id <id>]")
  print("defaultAction=register-enable-select-inputia")
}

@main
struct InputiaTISTool {
  static func main() {
    let args = CommandLine.arguments.dropFirst()
    if args.contains("--help") || args.contains("-h") {
      printUsage()
      return
    }
    if args.contains("--dump") {
      dump()
      return
    }
    if args.contains("--dump-current-input-source") {
      dumpCurrentInputSource()
      return
    }
    if let selectInputiaSourceIDIndex = args.firstIndex(of: "--select-inputia-source-id") {
      let sourceIDIndex = args.index(after: selectInputiaSourceIDIndex)
      guard sourceIDIndex < args.endIndex else {
        print("selectInputiaSourceIDMissing=true")
        return
      }
      select(inputSourceID: String(args[sourceIDIndex]))
      return
    }
    if let selectSourceIDIndex = args.firstIndex(of: "--select-source-id") {
      let sourceIDIndex = args.index(after: selectSourceIDIndex)
      guard sourceIDIndex < args.endIndex else {
        print("selectSourceIDMissing=true")
        return
      }
      selectSourceByID(inputSourceID: String(args[sourceIDIndex]))
      return
    }
    if args.contains("--reset-enable") {
      disable(inputSourceID: primaryModeIdentifier, label: "primary")
      disable(inputSourceID: bundleIdentifier, label: "parent")
      register(appPath: defaultAppPath)
      enable(inputSourceID: bundleIdentifier, label: "parent")
      enable(inputSourceID: primaryModeIdentifier, label: "primary")
      dump()
      select(inputSourceID: primaryModeIdentifier)
      dump()
      return
    }
    if !args.isEmpty {
      print("unknownCommand=\(args.joined(separator: " "))")
      printUsage()
      return
    }

    register(appPath: defaultAppPath)
    enable(inputSourceID: bundleIdentifier, label: "parent")
    enable(inputSourceID: primaryModeIdentifier, label: "primary")
    dump()
    select(inputSourceID: primaryModeIdentifier)
    dump()
  }
}
