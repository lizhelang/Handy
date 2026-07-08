import Darwin
import Foundation

@main
struct InputiaSettingsWindowSelfCheck {
  static func main() {
    let expectedSchemas: [(String, String)] = [
      ("中文全拼", "luna_pinyin_simp"),
      ("自然码双拼", "double_pinyin"),
      ("小鹤双拼", "double_pinyin_flypy"),
      ("搜狗双拼", "double_pinyin_sogou"),
      ("国标双拼", "guobiao_bispell"),
      ("微软双拼", "double_pinyin_mspy"),
      ("智能 ABC 双拼", "double_pinyin_abc"),
      ("拼音加加双拼", "double_pinyin_pyjj"),
      ("四通双拼", "double_pinyin_st"),
    ]
    let schemaPairs = inputiaSchemaOptions.map { ($0.title, $0.schemaId) }
    let schemaIds = inputiaSchemaOptions.map(\.schemaId)
    let schemaTitles = inputiaSchemaOptions.map(\.title)
    let inputModeShortcutValues = inputiaInputModeShortcutOptions.map(\.value)
    let scriptShortcutValues = inputiaScriptShortcutOptions.map(\.value)
    let tempSettingsURL = URL(fileURLWithPath: NSTemporaryDirectory())
      .appendingPathComponent("inputia-settings-window-self-check")
      .appendingPathComponent("settings.json")
    let defaultDocument = InputiaSettingsDocument.defaultDocument(for: tempSettingsURL)

    var dirtyDocument = defaultDocument
    dirtyDocument.schemaId = " "
    dirtyDocument.candidatePageSize = 99
    dirtyDocument.inputModeToggleShortcut = "bad"
    dirtyDocument.chineseScript = "bad"
    dirtyDocument.scriptToggleShortcut = "bad"
    dirtyDocument.punctuationPreference = "bad"
    dirtyDocument.characterWidthPreference = "bad"
    dirtyDocument.sensitiveBundleIds = []
    dirtyDocument.sanitize(using: tempSettingsURL)

    let checks: [(String, Bool)] = [
      ("settingsWindowProductTitleIsInputia", inputiaSettingsProductTitle == "Inputia"),
      ("settingsWindowTitleIsStable", inputiaSettingsWindowTitle == "Inputia 设置"),
      ("settingsWindowTitleHasNoSimplifiedSuffix", !inputiaSettingsWindowTitle.contains("简体")),
      (
        "settingsWindowSchemaListMatchesExpected",
        schemaPairs.elementsEqual(expectedSchemas) { lhs, rhs in
          lhs.0 == rhs.0 && lhs.1 == rhs.1
        }
      ),
      ("settingsWindowSchemaIdsUnique", Set(schemaIds).count == schemaIds.count),
      ("settingsWindowSchemaTitlesUnique", Set(schemaTitles).count == schemaTitles.count),
      ("settingsWindowHasNineSchemas", inputiaSchemaOptions.count == 9),
      ("settingsWindowHasGuobiaoBispell", schemaIds.contains("guobiao_bispell")),
      ("settingsWindowHasSogouDoublePinyin", schemaIds.contains("double_pinyin_sogou")),
      ("settingsWindowHasNaturalDoublePinyin", schemaIds.contains("double_pinyin")),
      (
        "settingsWindowDefaultCandidateCountIsSeven",
        inputiaSettingsDefaultCandidatePageSize == 7
      ),
      (
        "settingsWindowCandidateRangeIsOneToNine",
        inputiaSettingsMinCandidatePageSize == 1 && inputiaSettingsMaxCandidatePageSize == 9
      ),
      ("settingsWindowDefaultDocumentCandidateCountIsSeven", defaultDocument.candidatePageSize == 7),
      ("settingsWindowDefaultDocumentSchemaIsFullPinyin", defaultDocument.schemaId == "luna_pinyin_simp"),
      (
        "settingsWindowDefaultDocumentUsesShiftToggle",
        defaultDocument.inputModeToggleShortcut == "shift"
      ),
      (
        "settingsWindowDefaultDocumentUsesEnglishPunctuation",
        defaultDocument.punctuationPreference == "english_in_chinese"
      ),
      ("settingsWindowDefaultDocumentEnablesMemory", defaultDocument.memoryEnabled),
      ("settingsWindowDefaultDocumentEnablesPrivacyLearning", defaultDocument.privacyLearningEnabled),
      (
        "settingsWindowSensitiveDefaultsIncludeSecurityAgent",
        inputiaDefaultSensitiveBundleIds.contains("com.apple.SecurityAgent")
      ),
      (
        "settingsWindowSensitiveDefaultsIncludeSafariPrivate",
        inputiaDefaultSensitiveBundleIds.contains("com.apple.Safari.PrivateBrowsing")
      ),
      ("settingsWindowSanitizeClampsCandidateCount", dirtyDocument.candidatePageSize == 9),
      ("settingsWindowSanitizeBackfillsSchema", dirtyDocument.schemaId == "luna_pinyin_simp"),
      (
        "settingsWindowSanitizeBackfillsShortcuts",
        dirtyDocument.inputModeToggleShortcut == "shift"
          && dirtyDocument.scriptToggleShortcut == "control_shift_s"
      ),
      (
        "settingsWindowSanitizeBackfillsPreferences",
        dirtyDocument.punctuationPreference == "english_in_chinese"
          && dirtyDocument.characterWidthPreference == "half_width"
      ),
      (
        "settingsWindowSanitizeBackfillsSensitiveApps",
        dirtyDocument.sensitiveBundleIds == inputiaDefaultSensitiveBundleIds
      ),
      (
        "settingsWindowInputModeShortcutOptionsStable",
        inputModeShortcutValues == ["shift", "control_space", "none"]
      ),
      (
        "settingsWindowScriptShortcutOptionsStable",
        scriptShortcutValues == ["control_shift_s", "none"]
      ),
      (
        "settingsWindowFullPinyinSupportsSpellingCorrection",
        inputiaSchemaSupportsSpellingCorrection("luna_pinyin_simp")
      ),
      (
        "settingsWindowDoublePinyinDisablesSpellingCorrection",
        !inputiaSchemaSupportsSpellingCorrection("double_pinyin")
      ),
      (
        "settingsWindowGuobiaoDisablesSpellingCorrection",
        !inputiaSchemaSupportsSpellingCorrection("guobiao_bispell")
      ),
    ]

    let ok = checks.allSatisfy { $0.1 }
    print("settingsWindowSelfCheck=\(ok)")
    print("settingsWindowSchemaCount=\(inputiaSchemaOptions.count)")
    print("settingsWindowDefaultCandidateCount=\(inputiaSettingsDefaultCandidatePageSize)")
    print("settingsWindowCandidateMin=\(inputiaSettingsMinCandidatePageSize)")
    print("settingsWindowCandidateMax=\(inputiaSettingsMaxCandidatePageSize)")
    for (name, passed) in checks {
      print("\(name)=\(passed)")
    }
    exit(ok ? 0 : 1)
  }
}
