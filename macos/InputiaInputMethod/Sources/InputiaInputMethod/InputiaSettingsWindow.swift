import AppKit

struct InputiaSchemaOption {
  let title: String
  let schemaId: String
}

struct InputiaShortcutOption {
  let title: String
  let value: String
}

private struct InputiaMenuIconOption {
  let title: String
  let variant: String
}

let inputiaSettingsProductTitle = "Inputia"
let inputiaSettingsWindowTitle = "Inputia 设置"
let inputiaSettingsDefaultCandidatePageSize = 7
let inputiaSettingsMinCandidatePageSize = 1
let inputiaSettingsMaxCandidatePageSize = 9

let inputiaSchemaOptions = [
  InputiaSchemaOption(title: "中文全拼", schemaId: "luna_pinyin_simp"),
  InputiaSchemaOption(title: "自然码双拼", schemaId: "double_pinyin"),
  InputiaSchemaOption(title: "小鹤双拼", schemaId: "double_pinyin_flypy"),
  InputiaSchemaOption(title: "搜狗双拼", schemaId: "double_pinyin_sogou"),
  InputiaSchemaOption(title: "国标双拼", schemaId: "guobiao_bispell"),
  InputiaSchemaOption(title: "微软双拼", schemaId: "double_pinyin_mspy"),
  InputiaSchemaOption(title: "智能 ABC 双拼", schemaId: "double_pinyin_abc"),
  InputiaSchemaOption(title: "拼音加加双拼", schemaId: "double_pinyin_pyjj"),
  InputiaSchemaOption(title: "四通双拼", schemaId: "double_pinyin_st"),
]

let inputiaInputModeShortcutOptions = [
  InputiaShortcutOption(title: "Shift", value: "shift"),
  InputiaShortcutOption(title: "Control + Space", value: "control_space"),
  InputiaShortcutOption(title: "关闭", value: "none"),
]

let inputiaScriptShortcutOptions = [
  InputiaShortcutOption(title: "Control + Shift + S", value: "control_shift_s"),
  InputiaShortcutOption(title: "关闭", value: "none"),
]

let inputiaDefaultSensitiveBundleIds = [
  "com.1password.1password",
  "com.agilebits.onepassword7",
  "com.apple.Safari.PrivateBrowsing",
  "com.apple.SecurityAgent",
  "com.bitwarden.desktop",
  "com.lastpass.LastPass",
  "com.protonmail.protonmail",
]

private let inputiaMenuIconOptions = [
  InputiaMenuIconOption(title: "连珠 16 颗", variant: "pearl_16"),
  InputiaMenuIconOption(title: "连珠 14 颗", variant: "pearl_14"),
  InputiaMenuIconOption(title: "连珠 12 颗", variant: "pearl_12"),
  InputiaMenuIconOption(title: "连珠 18 颗", variant: "pearl_18"),
]

private func inputiaIsKnownMenuIconVariant(_ variant: String?) -> Bool {
  guard let variant else {
    return false
  }
  return inputiaMenuIconOptions.contains { $0.variant == variant }
}

func inputiaSchemaSupportsSpellingCorrection(_ schemaId: String) -> Bool {
  [
    "luna_pinyin",
    "luna_pinyin_simp",
    "luna_pinyin_tw",
    "luna_pinyin_fluency",
    "luna_quanpin",
  ].contains(schemaId)
}

struct InputiaSettingsDocument: Codable {
  var schemaId: String
  var candidatePageSize: Int
  var candidateFontSize: Int?
  var menuIconVariant: String?
  var shiftToggleEnabled: Bool
  var inputModeToggleShortcut: String?
  var chineseScript: String?
  var scriptToggleShortcut: String?
  var punctuationPreference: String
  var characterWidthPreference: String?
  var spellingCorrectionEnabled: Bool?
  var memoryEnabled: Bool
  var privacyLearningEnabled: Bool
  var sensitiveBundleIds: [String]
  var rimeDylibPath: String?
  var rimeSharedDataDir: String?
  var rimeUserDataDir: String?
  var memoryDbPath: String?

  enum CodingKeys: String, CodingKey {
    case schemaId = "schema_id"
    case candidatePageSize = "candidate_page_size"
    case candidateFontSize = "candidate_font_size"
    case menuIconVariant = "menu_icon_variant"
    case shiftToggleEnabled = "shift_toggle_enabled"
    case inputModeToggleShortcut = "input_mode_toggle_shortcut"
    case chineseScript = "chinese_script"
    case scriptToggleShortcut = "script_toggle_shortcut"
    case punctuationPreference = "punctuation_preference"
    case characterWidthPreference = "character_width_preference"
    case spellingCorrectionEnabled = "spelling_correction_enabled"
    case memoryEnabled = "memory_enabled"
    case privacyLearningEnabled = "privacy_learning_enabled"
    case sensitiveBundleIds = "sensitive_bundle_ids"
    case rimeDylibPath = "rime_dylib_path"
    case rimeSharedDataDir = "rime_shared_data_dir"
    case rimeUserDataDir = "rime_user_data_dir"
    case memoryDbPath = "memory_db_path"
  }

  static func defaultDocument(for settingsURL: URL) -> Self {
    let baseURL = settingsURL.deletingLastPathComponent()
    return InputiaSettingsDocument(
      schemaId: "luna_pinyin_simp",
      candidatePageSize: inputiaSettingsDefaultCandidatePageSize,
      candidateFontSize: 14,
      menuIconVariant: "pearl_16",
      shiftToggleEnabled: true,
      inputModeToggleShortcut: "shift",
      chineseScript: "simplified",
      scriptToggleShortcut: "control_shift_s",
      punctuationPreference: "english_in_chinese",
      characterWidthPreference: "half_width",
      spellingCorrectionEnabled: true,
      memoryEnabled: true,
      privacyLearningEnabled: true,
      sensitiveBundleIds: inputiaDefaultSensitiveBundleIds,
      rimeDylibPath: nil,
      rimeSharedDataDir: Self.bundledRimeDataPath(),
      rimeUserDataDir: baseURL.appendingPathComponent("rime", isDirectory: true).path,
      memoryDbPath: baseURL.appendingPathComponent("inputia_memory.db").path
    )
  }

  mutating func sanitize(using settingsURL: URL) {
    if schemaId.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
      schemaId = "luna_pinyin_simp"
    }
    candidatePageSize = max(
      inputiaSettingsMinCandidatePageSize,
      min(candidatePageSize, inputiaSettingsMaxCandidatePageSize)
    )
    candidateFontSize = max(12, min(candidateFontSize ?? 14, 22))
    if !inputiaIsKnownMenuIconVariant(menuIconVariant) {
      menuIconVariant = "pearl_16"
    }
    if inputModeToggleShortcut == nil {
      inputModeToggleShortcut = shiftToggleEnabled ? "shift" : "none"
    }
    if !["shift", "control_space", "none"].contains(inputModeToggleShortcut ?? "") {
      inputModeToggleShortcut = "shift"
    }
    shiftToggleEnabled = inputModeToggleShortcut == "shift"
    if chineseScript != "simplified" && chineseScript != "traditional" {
      chineseScript = "simplified"
    }
    if !["control_shift_s", "none"].contains(scriptToggleShortcut ?? "") {
      scriptToggleShortcut = "control_shift_s"
    }
    if punctuationPreference != "follow_input_mode" && punctuationPreference != "english_in_chinese" {
      punctuationPreference = "english_in_chinese"
    }
    if characterWidthPreference != "half_width" && characterWidthPreference != "full_width" {
      characterWidthPreference = "half_width"
    }
    if spellingCorrectionEnabled == nil {
      spellingCorrectionEnabled = true
    }
    if sensitiveBundleIds.isEmpty {
      sensitiveBundleIds = Self.defaultDocument(for: settingsURL).sensitiveBundleIds
    }
    let baseURL = settingsURL.deletingLastPathComponent()
    if rimeUserDataDir == nil {
      rimeUserDataDir = baseURL.appendingPathComponent("rime", isDirectory: true).path
    }
    if rimeSharedDataDir == nil || !FileManager.default.fileExists(atPath: rimeSharedDataDir ?? "") {
      rimeSharedDataDir = Self.bundledRimeDataPath()
    }
    if memoryDbPath == nil {
      memoryDbPath = baseURL.appendingPathComponent("inputia_memory.db").path
    }
    if InputiaProfile.current.isCandidate {
      rimeUserDataDir = InputiaProfile.current.rime.path
      memoryDbPath = InputiaProfile.current.memory.path
      rimeSharedDataDir = Self.bundledRimeDataPath()
    }
  }

  private static func bundledRimeDataPath() -> String? {
    var candidates = [Bundle.main.resourceURL?.appendingPathComponent("RimeData", isDirectory: true)].compactMap { $0 }
    if !InputiaProfile.current.isCandidate {
      candidates.append(URL(fileURLWithPath: "/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/RimeData", isDirectory: true))
    }

    for url in candidates where FileManager.default.fileExists(atPath: url.path) {
      return url.path
    }
    return nil
  }
}

final class InputiaSettingsWindowController: NSWindowController {
  private let settingsURL: URL
  private var settingsDocument: InputiaSettingsDocument
  private var settingsEdit: InputiaSettingsEdit?
  private var displayedDocument: InputiaSettingsDocument?
  private var pendingExternalImport: InputiaSettingsStore.ImportOperation?
  private var statusTimer: Timer?
  private var confirmationExpiryTimer: Timer?
  private var windowActivationObserver: NSObjectProtocol?
  private var statusIsError = false
  private let externalImportButton = NSButton(title: "查看并导入外部修改", target: nil, action: nil)

  private let schemaPopup = NSPopUpButton(frame: .zero, pullsDown: false)
  private let menuIconPopup = NSPopUpButton(frame: .zero, pullsDown: false)
  private let inputModeShortcutPopup = NSPopUpButton(frame: .zero, pullsDown: false)
  private let chineseScriptSegment = NSSegmentedControl(
    labels: ["简体", "繁体"],
    trackingMode: .selectOne,
    target: nil,
    action: nil
  )
  private let scriptShortcutPopup = NSPopUpButton(frame: .zero, pullsDown: false)
  private let candidateStepper = NSStepper()
  private let candidateCountField = NSTextField(labelWithString: "")
  private let candidateFontStepper = NSStepper()
  private let candidateFontSizeField = NSTextField(labelWithString: "")
  private let englishPunctuationCheckbox = NSButton(checkboxWithTitle: "中文模式始终使用英文标点", target: nil, action: nil)
  private let fullWidthCheckbox = NSButton(checkboxWithTitle: "使用全角输入", target: nil, action: nil)
  private let spellingCorrectionCheckbox = NSButton(checkboxWithTitle: "启用全拼纠错", target: nil, action: nil)
  private let memoryCheckbox = NSButton(checkboxWithTitle: "启用本地记忆排序", target: nil, action: nil)
  private let privacyCheckbox = NSButton(checkboxWithTitle: "允许隐私学习", target: nil, action: nil)
  private let sensitiveAppsTextView = NSTextView()
  private let importStatusLabel = NSTextField(labelWithString: "")
  private let statusLabel = NSTextField(labelWithString: "")

  init() {
    settingsURL = InputiaProfile.current.settings
    settingsDocument = InputiaSettingsDocument.defaultDocument(for: settingsURL)

    let window = NSWindow(
      contentRect: NSRect(x: 0, y: 0, width: 620, height: 700),
      styleMask: [.titled, .closable, .miniaturizable],
      backing: .buffered,
      defer: false
    )
    window.title = inputiaSettingsWindowTitle
    window.minSize = NSSize(width: 620, height: 700)
    window.center()
    super.init(window: window)
    window.contentView = makeContentView()
    reloadDocument()
    statusTimer = Timer.scheduledTimer(withTimeInterval: 0.75, repeats: true) { [weak self] _ in self?.refreshApplicationStatus() }
    windowActivationObserver = NotificationCenter.default.addObserver(forName: NSWindow.didBecomeKeyNotification, object: window, queue: .main) { [weak self] _ in self?.refreshApplicationStatus() }
  }

  deinit {
    statusTimer?.invalidate(); confirmationExpiryTimer?.invalidate()
    if let observer = windowActivationObserver { NotificationCenter.default.removeObserver(observer) }
  }

  @available(*, unavailable)
  required init?(coder: NSCoder) {
    nil
  }

  override func showWindow(_ sender: Any?) {
    if settingsEdit?.pending == nil && pendingExternalImport == nil { reloadDocument() }
    super.showWindow(sender)
    refreshApplicationStatus()
  }

  private func acceptSnapshot(_ snapshot: InputiaSettingsStore.Snapshot) throws {
    var document = try snapshot.decode(InputiaSettingsDocument.self)
    let sensitive = document.sensitiveBundleIds
    document.sanitize(using: settingsURL)
    document.sensitiveBundleIds = sensitive
    settingsDocument = document
    settingsEdit = InputiaSettingsEdit(snapshot)
    statusIsError = false
    externalImportButton.isHidden = true
    InputiaSettingsCache.shared(path: settingsURL.path).publish(snapshot)
    applyDocumentToControls()
  }

  private func reloadDocument() {
    do {
      try InputiaProfile.current.validateSettingsPath(settingsURL.path)
      try acceptSnapshot(InputiaSettingsStore.read(path: settingsURL.path))
    } catch {
      settingsEdit = nil
      applyDocumentToControls()
      showFailure(error)
    }
  }

  private func makeContentView() -> NSView {
    let root = NSView()
    root.translatesAutoresizingMaskIntoConstraints = false
    for checkbox in [
      englishPunctuationCheckbox,
      fullWidthCheckbox,
      spellingCorrectionCheckbox,
      memoryCheckbox,
      privacyCheckbox,
    ] {
      checkbox.target = self
      checkbox.action = #selector(controlChanged)
    }

    let scrollView = NSScrollView()
    scrollView.translatesAutoresizingMaskIntoConstraints = false
    scrollView.hasVerticalScroller = true
    scrollView.drawsBackground = false
    scrollView.borderType = .noBorder

    let contentView = NSView()
    contentView.translatesAutoresizingMaskIntoConstraints = false

    let title = NSTextField(labelWithString: inputiaSettingsProductTitle)
    title.font = .systemFont(ofSize: 24, weight: .semibold)

    let subtitle = NSTextField(labelWithString: "输入、语音、剪贴板")
    subtitle.textColor = .secondaryLabelColor
    subtitle.font = .systemFont(ofSize: 12)

    let stack = NSStackView()
    stack.orientation = .vertical
    stack.alignment = .leading
    stack.spacing = 14
    stack.translatesAutoresizingMaskIntoConstraints = false
    stack.addArrangedSubview(title)
    stack.addArrangedSubview(subtitle)
    stack.addArrangedSubview(makeSeparator())
    stack.addArrangedSubview(makeSchemaRow())
    stack.addArrangedSubview(makeMenuIconRow())
    stack.addArrangedSubview(makeInputModeShortcutRow())
    stack.addArrangedSubview(makeChineseScriptRow())
    stack.addArrangedSubview(makeScriptShortcutRow())
    stack.addArrangedSubview(makeCandidateRow())
    stack.addArrangedSubview(makeCandidateFontRow())
    stack.addArrangedSubview(englishPunctuationCheckbox)
    stack.addArrangedSubview(fullWidthCheckbox)
    stack.addArrangedSubview(spellingCorrectionCheckbox)
    stack.addArrangedSubview(makeSeparator())
    stack.addArrangedSubview(memoryCheckbox)
    stack.addArrangedSubview(privacyCheckbox)
    stack.addArrangedSubview(makeImportSection())
    stack.addArrangedSubview(makeSeparator())
    stack.addArrangedSubview(makeSensitiveAppsSection())
    stack.addArrangedSubview(makeSeparator())
    stack.addArrangedSubview(makeFooter())

    contentView.addSubview(stack)
    scrollView.documentView = contentView
    root.addSubview(scrollView)
    NSLayoutConstraint.activate([
      scrollView.leadingAnchor.constraint(equalTo: root.leadingAnchor),
      scrollView.trailingAnchor.constraint(equalTo: root.trailingAnchor),
      scrollView.topAnchor.constraint(equalTo: root.topAnchor),
      scrollView.bottomAnchor.constraint(equalTo: root.bottomAnchor),
      contentView.leadingAnchor.constraint(equalTo: scrollView.contentView.leadingAnchor),
      contentView.trailingAnchor.constraint(equalTo: scrollView.contentView.trailingAnchor),
      contentView.topAnchor.constraint(equalTo: scrollView.contentView.topAnchor),
      contentView.widthAnchor.constraint(equalTo: scrollView.contentView.widthAnchor),
      stack.leadingAnchor.constraint(equalTo: contentView.leadingAnchor, constant: 24),
      stack.trailingAnchor.constraint(equalTo: contentView.trailingAnchor, constant: -24),
      stack.topAnchor.constraint(equalTo: contentView.topAnchor, constant: 22),
      stack.bottomAnchor.constraint(equalTo: contentView.bottomAnchor, constant: -22),
    ])
    return root
  }

  private func makeSchemaRow() -> NSView {
    for option in inputiaSchemaOptions {
      schemaPopup.addItem(withTitle: option.title)
      schemaPopup.lastItem?.representedObject = option.schemaId
    }
    schemaPopup.target = self
    schemaPopup.action = #selector(controlChanged)
    return labeledRow(title: "输入方案", control: schemaPopup)
  }

  private func makeInputModeShortcutRow() -> NSView {
    for option in inputiaInputModeShortcutOptions {
      inputModeShortcutPopup.addItem(withTitle: option.title)
      inputModeShortcutPopup.lastItem?.representedObject = option.value
    }
    inputModeShortcutPopup.target = self
    inputModeShortcutPopup.action = #selector(controlChanged)
    return labeledRow(title: "中英切换", control: inputModeShortcutPopup)
  }

  private func makeMenuIconRow() -> NSView {
    for option in inputiaMenuIconOptions {
      menuIconPopup.addItem(withTitle: option.title)
      menuIconPopup.lastItem?.representedObject = option.variant
    }
    menuIconPopup.target = self
    menuIconPopup.action = #selector(controlChanged)
    return labeledRow(title: "菜单栏图标", control: menuIconPopup)
  }

  private func makeChineseScriptRow() -> NSView {
    chineseScriptSegment.target = self
    chineseScriptSegment.action = #selector(controlChanged)
    chineseScriptSegment.setWidth(82, forSegment: 0)
    chineseScriptSegment.setWidth(82, forSegment: 1)
    return labeledRow(title: "中文字形", control: chineseScriptSegment)
  }

  private func makeScriptShortcutRow() -> NSView {
    for option in inputiaScriptShortcutOptions {
      scriptShortcutPopup.addItem(withTitle: option.title)
      scriptShortcutPopup.lastItem?.representedObject = option.value
    }
    scriptShortcutPopup.target = self
    scriptShortcutPopup.action = #selector(controlChanged)
    return labeledRow(title: "简繁切换", control: scriptShortcutPopup)
  }

  private func makeCandidateRow() -> NSView {
    candidateStepper.minValue = Double(inputiaSettingsMinCandidatePageSize)
    candidateStepper.maxValue = Double(inputiaSettingsMaxCandidatePageSize)
    candidateStepper.increment = 1
    candidateStepper.target = self
    candidateStepper.action = #selector(candidateStepperChanged)
    candidateCountField.alignment = .right
    candidateCountField.widthAnchor.constraint(equalToConstant: 26).isActive = true

    let row = NSStackView(views: [candidateCountField, candidateStepper])
    row.orientation = .horizontal
    row.spacing = 8
    row.alignment = .centerY
    return labeledRow(title: "候选数量", control: row)
  }

  private func makeCandidateFontRow() -> NSView {
    candidateFontStepper.minValue = 12
    candidateFontStepper.maxValue = 22
    candidateFontStepper.increment = 1
    candidateFontStepper.target = self
    candidateFontStepper.action = #selector(candidateFontStepperChanged)
    candidateFontSizeField.alignment = .right
    candidateFontSizeField.widthAnchor.constraint(equalToConstant: 26).isActive = true

    let row = NSStackView(views: [candidateFontSizeField, candidateFontStepper])
    row.orientation = .horizontal
    row.spacing = 8
    row.alignment = .centerY
    return labeledRow(title: "候选字号", control: row)
  }

  private func makeImportSection() -> NSView {
    let label = NSTextField(labelWithString: "历史导入")
    label.font = .systemFont(ofSize: 13, weight: .medium)

    let historyButton = NSButton(title: "导入语音历史", target: self, action: #selector(importHandyHistory))
    historyButton.bezelStyle = .rounded

    let clipboardButton = NSButton(title: "导入剪贴板历史", target: self, action: #selector(importHandyClipboard))
    clipboardButton.bezelStyle = .rounded

    let allButton = NSButton(title: "全部导入", target: self, action: #selector(importAllHandyHistory))
    allButton.bezelStyle = .rounded

    let buttonRow = NSStackView(views: [historyButton, clipboardButton, allButton])
    buttonRow.orientation = .horizontal
    buttonRow.alignment = .centerY
    buttonRow.spacing = 10

    importStatusLabel.textColor = .secondaryLabelColor
    importStatusLabel.lineBreakMode = .byTruncatingTail
    importStatusLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    importStatusLabel.widthAnchor.constraint(equalToConstant: 512).isActive = true

    let stack = NSStackView(views: [label, buttonRow, importStatusLabel])
    stack.orientation = .vertical
    stack.alignment = .leading
    stack.spacing = 8
    stack.widthAnchor.constraint(equalToConstant: 512).isActive = true
    return stack
  }

  private func makeSensitiveAppsSection() -> NSView {
    let label = NSTextField(labelWithString: "敏感 App 不学习")
    label.font = .systemFont(ofSize: 13, weight: .medium)

    let scrollView = NSScrollView()
    scrollView.hasVerticalScroller = true
    scrollView.borderType = .bezelBorder
    scrollView.documentView = sensitiveAppsTextView
    scrollView.translatesAutoresizingMaskIntoConstraints = false
    sensitiveAppsTextView.font = .monospacedSystemFont(ofSize: 12, weight: .regular)
    sensitiveAppsTextView.isRichText = false
    sensitiveAppsTextView.allowsUndo = true

    let stack = NSStackView(views: [label, scrollView])
    stack.orientation = .vertical
    stack.alignment = .leading
    stack.spacing = 8
    NSLayoutConstraint.activate([
      scrollView.widthAnchor.constraint(equalToConstant: 512),
      scrollView.heightAnchor.constraint(equalToConstant: 120),
    ])
    return stack
  }

  private func makeFooter() -> NSView {
    externalImportButton.target = self
    externalImportButton.action = #selector(previewExternalSettings)
    externalImportButton.bezelStyle = .rounded
    externalImportButton.isHidden = true
    let saveButton = NSButton(title: "保存设置", target: self, action: #selector(saveSettings))
    saveButton.bezelStyle = .rounded
    saveButton.keyEquivalent = "\r"
    saveButton.controlSize = .regular
    saveButton.translatesAutoresizingMaskIntoConstraints = false

    let revealButton = NSButton(title: "打开配置文件夹", target: self, action: #selector(openSettingsFolder))
    revealButton.bezelStyle = .rounded
    revealButton.controlSize = .regular
    revealButton.translatesAutoresizingMaskIntoConstraints = false

    let licensesButton = NSButton(title: "开源许可", target: self, action: #selector(openOpenSourceLicenses))
    licensesButton.bezelStyle = .rounded
    licensesButton.controlSize = .regular
    licensesButton.translatesAutoresizingMaskIntoConstraints = false

    statusLabel.textColor = .secondaryLabelColor
    statusLabel.lineBreakMode = .byTruncatingMiddle
    statusLabel.translatesAutoresizingMaskIntoConstraints = false
    statusLabel.maximumNumberOfLines = 2
    statusLabel.setContentHuggingPriority(.defaultLow, for: .horizontal)
    statusLabel.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)
    saveButton.setContentHuggingPriority(.required, for: .horizontal)
    saveButton.setContentCompressionResistancePriority(.required, for: .horizontal)
    revealButton.setContentHuggingPriority(.required, for: .horizontal)
    revealButton.setContentCompressionResistancePriority(.required, for: .horizontal)
    licensesButton.setContentHuggingPriority(.required, for: .horizontal)
    licensesButton.setContentCompressionResistancePriority(.required, for: .horizontal)

    let spacer = NSView()
    spacer.translatesAutoresizingMaskIntoConstraints = false
    spacer.setContentHuggingPriority(.defaultLow, for: .horizontal)
    spacer.setContentCompressionResistancePriority(.defaultLow, for: .horizontal)

    let buttonRow = NSStackView(views: [licensesButton, spacer, revealButton, saveButton])
    buttonRow.orientation = .horizontal
    buttonRow.alignment = .centerY
    buttonRow.spacing = 10
    buttonRow.distribution = .fill
    buttonRow.detachesHiddenViews = false
    buttonRow.translatesAutoresizingMaskIntoConstraints = false

    let footer = NSStackView(views: [statusLabel, externalImportButton, buttonRow])
    footer.orientation = .vertical
    footer.alignment = .leading
    footer.spacing = 8
    footer.distribution = .fill
    footer.translatesAutoresizingMaskIntoConstraints = false

    NSLayoutConstraint.activate([
      footer.widthAnchor.constraint(equalToConstant: 512),
      statusLabel.widthAnchor.constraint(equalTo: footer.widthAnchor),
      buttonRow.widthAnchor.constraint(equalTo: footer.widthAnchor),
      saveButton.widthAnchor.constraint(greaterThanOrEqualToConstant: 112),
      revealButton.widthAnchor.constraint(greaterThanOrEqualToConstant: 168),
      licensesButton.widthAnchor.constraint(greaterThanOrEqualToConstant: 96),
      saveButton.heightAnchor.constraint(greaterThanOrEqualToConstant: 30),
      revealButton.heightAnchor.constraint(greaterThanOrEqualToConstant: 30),
      licensesButton.heightAnchor.constraint(greaterThanOrEqualToConstant: 30),
    ])
    return footer
  }

  private func labeledRow(title: String, control: NSView) -> NSView {
    let label = NSTextField(labelWithString: title)
    label.font = .systemFont(ofSize: 13, weight: .medium)
    label.widthAnchor.constraint(equalToConstant: 96).isActive = true

    let row = NSStackView(views: [label, control])
    row.orientation = .horizontal
    row.alignment = .centerY
    row.spacing = 12
    row.widthAnchor.constraint(equalToConstant: 512).isActive = true
    return row
  }

  private func makeSeparator() -> NSBox {
    let separator = NSBox()
    separator.boxType = .separator
    separator.widthAnchor.constraint(equalToConstant: 512).isActive = true
    return separator
  }

  private func applyDocumentToControls() {
    let schemaIndex = schemaPopup.itemArray.firstIndex {
      ($0.representedObject as? String) == settingsDocument.schemaId
    }
    if let schemaIndex {
      schemaPopup.selectItem(at: schemaIndex)
    }
    if schemaPopup.selectedItem == nil {
      schemaPopup.selectItem(at: 0)
    }
    let iconVariant = settingsDocument.menuIconVariant ?? "pearl_16"
    let iconIndex = menuIconPopup.itemArray.firstIndex {
      ($0.representedObject as? String) == iconVariant
    }
    if let iconIndex {
      menuIconPopup.selectItem(at: iconIndex)
    } else {
      menuIconPopup.selectItem(at: 0)
    }
    candidateStepper.integerValue = settingsDocument.candidatePageSize
    candidateCountField.stringValue = "\(settingsDocument.candidatePageSize)"
    let candidateFontSize = settingsDocument.candidateFontSize ?? 14
    candidateFontStepper.integerValue = candidateFontSize
    candidateFontSizeField.stringValue = "\(candidateFontSize)"
    let shortcut = settingsDocument.inputModeToggleShortcut ?? (settingsDocument.shiftToggleEnabled ? "shift" : "none")
    let shortcutIndex = inputModeShortcutPopup.itemArray.firstIndex {
      ($0.representedObject as? String) == shortcut
    }
    if let shortcutIndex {
      inputModeShortcutPopup.selectItem(at: shortcutIndex)
    } else {
      inputModeShortcutPopup.selectItem(at: 0)
    }
    chineseScriptSegment.selectedSegment = settingsDocument.chineseScript == "traditional" ? 1 : 0
    let scriptShortcut = settingsDocument.scriptToggleShortcut ?? "control_shift_s"
    let scriptShortcutIndex = scriptShortcutPopup.itemArray.firstIndex {
      ($0.representedObject as? String) == scriptShortcut
    }
    if let scriptShortcutIndex {
      scriptShortcutPopup.selectItem(at: scriptShortcutIndex)
    } else {
      scriptShortcutPopup.selectItem(at: 0)
    }
    englishPunctuationCheckbox.state = settingsDocument.punctuationPreference == "english_in_chinese" ? .on : .off
    fullWidthCheckbox.state = settingsDocument.characterWidthPreference == "full_width" ? .on : .off
    updateSpellingCorrectionAvailability(for: settingsDocument.schemaId)
    memoryCheckbox.state = settingsDocument.memoryEnabled ? .on : .off
    privacyCheckbox.state = settingsDocument.privacyLearningEnabled ? .on : .off
    sensitiveAppsTextView.string = settingsDocument.sensitiveBundleIds.joined(separator: "\n")
    importStatusLabel.stringValue = handyDataStatusText()
    statusLabel.stringValue = settingsURL.path
    displayedDocument = documentFromControls()
  }

  @objc private func candidateStepperChanged() {
    candidateCountField.stringValue = "\(candidateStepper.integerValue)"
    saveSettings()
  }

  @objc private func candidateFontStepperChanged() {
    candidateFontSizeField.stringValue = "\(candidateFontStepper.integerValue)"
    saveSettings()
  }

  @objc private func controlChanged() {
    saveSettings()
  }

  private func documentFromControls() -> InputiaSettingsDocument {
    var next = settingsDocument
    next.schemaId = (schemaPopup.selectedItem?.representedObject as? String) ?? "luna_pinyin_simp"
    next.menuIconVariant = (menuIconPopup.selectedItem?.representedObject as? String) ?? "pearl_16"
    next.candidatePageSize = candidateStepper.integerValue
    next.candidateFontSize = candidateFontStepper.integerValue
    next.inputModeToggleShortcut = (inputModeShortcutPopup.selectedItem?.representedObject as? String) ?? "shift"
    next.shiftToggleEnabled = next.inputModeToggleShortcut == "shift"
    next.chineseScript = chineseScriptSegment.selectedSegment == 1 ? "traditional" : "simplified"
    next.scriptToggleShortcut = (scriptShortcutPopup.selectedItem?.representedObject as? String) ?? "control_shift_s"
    next.punctuationPreference = englishPunctuationCheckbox.state == .on
      ? "english_in_chinese"
      : "follow_input_mode"
    next.characterWidthPreference = fullWidthCheckbox.state == .on ? "full_width" : "half_width"
    next.spellingCorrectionEnabled = inputiaSchemaSupportsSpellingCorrection(next.schemaId)
      && spellingCorrectionCheckbox.state == .on
    next.memoryEnabled = memoryCheckbox.state == .on
    next.privacyLearningEnabled = privacyCheckbox.state == .on
    next.sensitiveBundleIds = sensitiveAppsTextView.string
      .components(separatedBy: .newlines)
      .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) }
      .filter { !$0.isEmpty }
    return next
  }

  @objc private func saveSettings() { _ = persistControls() }

  @discardableResult
  private func persistControls() -> Bool {
    guard pendingExternalImport == nil else {
      showFailure(InputiaSettingsStore.Failure(code: "commit_uncertain")); return false
    }
    guard let edit = settingsEdit else {
      statusLabel.stringValue = "设置尚未成功读取，请重新打开窗口或修复文件"
      return false
    }
    let next = documentFromControls()

    do {
      try InputiaProfile.current.validateSettingsPath(settingsURL.path)
      let patch = try InputiaSettingsStore.dirtyPatch(from: displayedDocument ?? settingsDocument, to: next)
      guard let result = try edit.apply(path: settingsURL.path, patch: patch) else { return true }
      try acceptSnapshot(result.current)
      if result.status != "saved" {
        showFailure(InputiaSettingsStore.Failure(code: result.status))
        return false
      }
      statusLabel.stringValue = "已保存版本 \(result.commitRevision ?? result.current.revision)，等待输入会话确认"
      return true
    } catch {
      showFailure(error)
      return false
    }
  }

  private func showFailure(_ error: Error) {
    statusIsError = true
    statusLabel.stringValue = error.localizedDescription
    if let failure = error as? InputiaSettingsStore.Failure,
      ["external_edit", "external_changed"].contains(failure.code) { externalImportButton.isHidden = false }
  }

  private func refreshApplicationStatus() {
    confirmationExpiryTimer?.invalidate(); confirmationExpiryTimer = nil
    guard window?.isVisible == true, !statusIsError, let edit = settingsEdit,
      edit.pending == nil, pendingExternalImport == nil else { return }
    let state = InputiaSettingsCache.shared(path: settingsURL.path).state
    if let failure = state.failure { showFailure(InputiaSettingsStore.Failure(code: failure)); return }
    if let current = state.snapshot, current.identity != edit.base.identity {
      statusLabel.stringValue = "其他窗口已保存新版本；当前修改提交时会核对冲突"
      return
    }
    statusLabel.stringValue = state.application?.summary(for: edit.base)
      ?? "已保存版本 \(edit.base.revision)，等待输入会话确认"
    if let application = state.application {
      let remaining = application.expiresAt - ProcessInfo.processInfo.systemUptime
      if remaining > 0 {
        let timer = Timer(timeInterval: remaining, repeats: false) { [weak self] _ in self?.refreshApplicationStatus() }
        confirmationExpiryTimer = timer
        RunLoop.main.add(timer, forMode: .common)
      }
    }
  }

  @objc private func previewExternalSettings() {
    guard settingsEdit?.pending == nil else { showFailure(InputiaSettingsStore.Failure(code: "commit_uncertain")); return }
    do {
      if pendingExternalImport == nil {
        let external = try InputiaSettingsStore.inspectExternal(path: settingsURL.path)
        let data = try JSONSerialization.data(withJSONObject: external.values, options: [.prettyPrinted, .sortedKeys])
        let preview = NSTextView(frame: NSRect(x: 0, y: 0, width: 480, height: 240))
        preview.isEditable = false; preview.isRichText = false
        preview.string = String(decoding: data, as: UTF8.self)
        let scroll = NSScrollView(frame: preview.frame)
        scroll.hasVerticalScroller = true; scroll.documentView = preview
        let alert = NSAlert()
        alert.messageText = "导入外部修改的设置"
        alert.informativeText = settingsURL.path + "\n请核对下面的实际内容；确认后保存为新的设置版本。"
        alert.accessoryView = scroll
        alert.addButton(withTitle: "确认导入"); alert.addButton(withTitle: "取消")
        guard alert.runModal() == .alertFirstButtonReturn else { return }
        pendingExternalImport = .init(external)
      }
      guard let operation = pendingExternalImport else { return }
      let result = try InputiaSettingsStore.importExternal(path: settingsURL.path, operation: operation)
      pendingExternalImport = nil
      try acceptSnapshot(result.current)
      if result.status != "saved" { showFailure(InputiaSettingsStore.Failure(code: result.status)) }
      else { statusLabel.stringValue = "已导入版本 \(result.commitRevision ?? result.current.revision)，等待输入会话确认" }
    } catch {
      if let failure = error as? InputiaSettingsStore.Failure,
        ["invalid_request", "operation_mismatch", "revision_exhausted", "external_changed", "external_edit"].contains(failure.code) { pendingExternalImport = nil }
      showFailure(error)
    }
  }

  @objc private func importHandyHistory() {
    importHandySources(includeHistory: true, includeClipboard: false)
  }

  @objc private func importHandyClipboard() {
    importHandySources(includeHistory: false, includeClipboard: true)
  }

  @objc private func importAllHandyHistory() {
    importHandySources(includeHistory: true, includeClipboard: true)
  }

  private func importHandySources(includeHistory: Bool, includeClipboard: Bool) {
    guard persistControls() else { return }
    #if INPUTIA_PAIRED_BUILD
    let selection = includeHistory && includeClipboard ? "both" : (includeHistory ? "history" : "clipboard")
    importStatusLabel.stringValue = "正在查询导入状态…"
    InputiaMemoryImport.perform(selection: selection) { [weak self] status in self?.importStatusLabel.stringValue = status }
    #else
    let bridge = InputiaRustBridge.makeDefault()
    let result = InputiaHandyMemorySync.sync(
      importer: bridge,
      includeHistory: includeHistory,
      includeClipboard: includeClipboard
    )
    importStatusLabel.stringValue = "导入结果：" + result.summaryText
    #endif
  }

  private func handyDataStatusText() -> String {
    #if INPUTIA_PAIRED_BUILD
    return "由已认证的学习服务导入；未确认的操作将使用原记录恢复"
    #else
    return InputiaHandyMemorySync.statusText()
    #endif
  }

  @objc private func openSettingsFolder() {
    guard (try? InputiaProfile.current.validateSettingsPath(settingsURL.path)) != nil else { return }
    NSWorkspace.shared.activateFileViewerSelecting([settingsURL])
  }

  @objc private func openOpenSourceLicenses() {
    let candidates = [
      Bundle.main.resourceURL?.appendingPathComponent("InputiaOpenSourceLicenses.md"),
      URL(fileURLWithPath: "/Library/Input Methods/InputiaInputMethod.app/Contents/Resources/InputiaOpenSourceLicenses.md"),
    ].compactMap { $0 }

    for url in candidates where FileManager.default.fileExists(atPath: url.path) {
      NSWorkspace.shared.open(url)
      return
    }

    statusLabel.stringValue = "未找到开源许可文档"
  }

  private func updateSpellingCorrectionAvailability(for schemaId: String) {
    let supportsSpellingCorrection = inputiaSchemaSupportsSpellingCorrection(schemaId)
    spellingCorrectionCheckbox.isEnabled = supportsSpellingCorrection
    spellingCorrectionCheckbox.state = supportsSpellingCorrection && (settingsDocument.spellingCorrectionEnabled ?? true) ? .on : .off
  }
}
