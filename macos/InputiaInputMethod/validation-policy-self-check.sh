#!/bin/zsh
set -eu
set -o pipefail

ROOT_DIR="$(cd "$(dirname "$0")" && pwd)"

/usr/bin/python3 - "$ROOT_DIR" <<'PY'
from pathlib import Path
import sys

root = Path(sys.argv[1])


def read(name: str) -> str:
    return (root / name).read_text(encoding="utf-8")


def require(condition: bool, reason: str) -> None:
    if not condition:
        print(f"validationPolicySelfCheck=false reason={reason}")
        raise SystemExit(1)


dev_fast = read("dev-fast.sh")
rime_latency = read("rime-latency-self-check.sh")
verify_nongui = read("verify-nongui.sh")
install_check = read("install-check.sh")
install_handoff = read("install-handoff.sh")
repair_tis_duplicates = read("repair-tis-duplicates.sh")
build = read("build.sh")
verify_pkg = read("verify-pkg.sh")
settings_launcher_self_check = read("settings-launcher-build-self-check.sh")
full_check = read("release/full-check.sh")
candidate_panel = read("Sources/InputiaInputMethod/InputiaCandidatePanel.swift")
candidate_panel_self_check = read("Tools/InputiaCandidatePanelSelfCheck.swift")
settings_window_self_check = read("Tools/InputiaSettingsWindowSelfCheck.swift")
menu_readiness = read("menu-readiness.sh")
gui_readiness = read("gui-smoke-readiness.sh")
gui_suite = read("gui-smoke-suite.sh")
status = read("status.sh")
post_install = read("post-install-regression.sh")
tis = read("tis-readiness.sh")

require('validationTier=dev-fast' in dev_fast, "dev-fast-missing-tier-marker")
require('touchesMenuBar=false' in dev_fast, "dev-fast-missing-menu-policy")
require('opensGUI=false' in dev_fast, "dev-fast-missing-gui-policy")
require('changesSystemInputSource=false' in dev_fast, "dev-fast-missing-input-source-policy")
require('checksNotarization=false' in dev_fast, "dev-fast-missing-notarization-policy")
require('"$ROOT_DIR/validation-policy-self-check.sh"' in dev_fast, "dev-fast-missing-policy-self-check")
require('"$ROOT_DIR/rime-latency-self-check.sh"' in dev_fast, "dev-fast-missing-rime-latency-self-check")
require('"$ROOT_DIR/settings-launcher-build-self-check.sh"' in dev_fast, "dev-fast-missing-settings-launcher-build-self-check")
require('"$ROOT_DIR/build/inputia-candidate-panel-self-check"' in dev_fast, "dev-fast-missing-candidate-panel-self-check")
require('"$ROOT_DIR/build/inputia-settings-window-self-check"' in dev_fast, "dev-fast-missing-settings-window-self-check")
require('settingsWindowSelfCheck=\\(ok)' in settings_window_self_check, "settings-window-self-check-missing-pass-marker")
require('settingsWindowSchemaCount=\\(inputiaSchemaOptions.count)' in settings_window_self_check, "settings-window-self-check-missing-schema-count")
require('settingsWindowDefaultCandidateCount=\\(inputiaSettingsDefaultCandidatePageSize)' in settings_window_self_check, "settings-window-self-check-missing-candidate-count-output")
require('settingsWindowHasGuobiaoBispell' in settings_window_self_check, "settings-window-self-check-missing-guobiao-assertion")
require('settingsWindowHasSogouDoublePinyin' in settings_window_self_check, "settings-window-self-check-missing-sogou-assertion")
require('settingsWindowTitleHasNoSimplifiedSuffix' in settings_window_self_check, "settings-window-self-check-missing-title-assertion")
for forbidden in [
    "menu-readiness.sh",
    "gui-smoke-readiness.sh",
    "gui-smoke-suite.sh",
    "post-install-regression.sh",
    "status.sh",
    "notarization-readiness.sh",
    "smoke-textedit.sh",
    "smoke-safari-typing.sh",
    "smoke-clipboard-recall.sh",
]:
    require(forbidden not in dev_fast, f"dev-fast-unexpected-{forbidden}")

require('verifyNonguiCompatibilityMode=dev-fast' in verify_nongui, "verify-nongui-not-dev-fast-compatible")
require('exec "$ROOT_DIR/dev-fast.sh" "$@"' in verify_nongui, "verify-nongui-default-does-not-exec-dev-fast")
require('INPUTIA_VERIFY_NONGUI_FULL:-0' in verify_nongui, "verify-nongui-missing-full-opt-in")
require('verifyNonguiMenuReadinessCacheFile=' in verify_nongui, "verify-nongui-full-missing-menu-cache-output")
require('INPUTIA_MENU_READINESS_CACHE_FILE' in verify_nongui, "verify-nongui-full-missing-menu-cache")
require('VERIFY_TEMP_FILES+=("$INPUTIA_MENU_READINESS_CACHE_FILE")' in verify_nongui, "verify-nongui-full-menu-cache-not-cleaned")
require(
    verify_nongui.index('INPUTIA_MENU_READINESS_CACHE_FILE') < verify_nongui.index('INPUTIA_MENU_READINESS_ALLOW_AXPRESS'),
    "verify-nongui-menu-cache-after-axpress-opt-in",
)

require('validationTier=install-check' in install_check, "install-check-missing-tier-marker")
require('INPUTIA_TIS_INCLUDE_MENU_READINESS=0 "$ROOT_DIR/tis-readiness.sh"' in install_check, "install-check-must-disable-menu-readiness")
require('installCheckBlockReasons=' in install_check, "install-check-missing-block-reasons")
require('installCheckRequiredAction=' in install_check, "install-check-missing-required-action")
require('installCheckRequiredActions=' in install_check, "install-check-missing-required-actions")
require('installCheckRequiredCommands=present' in install_check, "install-check-missing-required-commands-present")
require('installCheckCommand.runInstallHandoff=' in install_check, "install-check-missing-handoff-command-output")
require('installCheckCommand.adminInstall=' in install_check, "install-check-missing-admin-install-command-output")
require('installCheckCommand.repairTISDuplicates=' in install_check, "install-check-missing-repair-command-output")
require('installCheckCommand.awaitSystemInstall=' in install_check, "install-check-missing-await-command-output")
require('installCheckCommand.verify=' in install_check, "install-check-missing-verify-command-output")
require('installHandoffCurrent=' in install_check, "install-check-missing-handoff-current")
require('installHandoffBlockReasons=' in install_check, "install-check-missing-handoff-block-reasons")
require('installHandoffRequiredAction=run-install-handoff' in install_check, "install-check-missing-handoff-required-action")
require('source-commit-mismatch' in install_check, "install-check-missing-handoff-source-commit-mismatch")
require('build-cdhash-mismatch' in install_check, "install-check-missing-handoff-build-cdhash-mismatch")
require('package-sha-mismatch' in install_check, "install-check-missing-handoff-package-sha-mismatch")
require('INPUTIA_INSTALL_CHECK_SELF_CHECK:-0' in install_check, "install-check-missing-self-check-mode")
require('installCheckSelfCheck=true' in install_check, "install-check-missing-self-check-pass-marker")
require('installHandoffFreshnessSelfCheck=true' in install_check, "install-check-missing-handoff-freshness-self-check")
require('installRequiredCommandsSelfCheck=true' in install_check, "install-check-missing-required-commands-self-check")
require('run-install-handoff-and-admin-install' in install_check, "install-check-missing-admin-required-action")
require('restart-inputia-host-after-install' in install_check, "install-check-missing-running-host-action")
require('tis-duplicate-matches' in install_check, "install-check-missing-tis-duplicate-blocker")
require('remove-duplicate-inputia-and-readd-once' in install_check, "install-check-missing-tis-duplicate-action")
require('run-repair-tis-duplicates' in install_check, "install-check-missing-tis-duplicate-actions-chain")
require('actions-mismatch' in install_check, "install-check-self-check-missing-actions-assertion")
require('command-keys-mismatch' in install_check, "install-check-self-check-missing-command-assertion")
require('buildSettingsExpectedHostCDHash=' in install_check, "install-check-missing-build-settings-expected-host-cdhash")
require('systemSettingsExpectedHostCDHash=' in install_check, "install-check-missing-system-settings-expected-host-cdhash")
require('"$system_settings_expected_host_cdhash" == "$build_cdhash"' in install_check, "install-check-settings-match-not-bound-to-host-cdhash")
require('"$ROOT_DIR/install-check.sh"' in dev_fast, "dev-fast-missing-install-check-self-check")
require('INPUTIA_INSTALL_CHECK_SELF_CHECK=1 "$ROOT_DIR/install-check.sh"' in dev_fast, "dev-fast-install-check-not-self-check-only")
require('installCheckBlockReasons=' in install_handoff, "install-handoff-missing-block-reasons")
require('installCheckRequiredAction=' in install_handoff, "install-handoff-missing-required-action")
require('installCheckRequiredActions=' in install_handoff, "install-handoff-missing-required-actions")
require('installCheckPassed=' in install_handoff, "install-handoff-missing-install-check-pass")
require('settingsMatchesBuild=' in install_handoff, "install-handoff-missing-settings-match-summary")
require('buildSettingsExpectedHostCDHash=' in install_handoff, "install-handoff-missing-build-settings-host-cdhash")
require('systemSettingsExpectedHostCDHash=' in install_handoff, "install-handoff-missing-system-settings-host-cdhash")
require('sourceBranch=' in install_handoff, "install-handoff-missing-source-branch")
require('sourceCommit=' in install_handoff, "install-handoff-missing-source-commit")
require('sourceUpstream=' in install_handoff, "install-handoff-missing-source-upstream")
require('sourceDirty=' in install_handoff, "install-handoff-missing-source-dirty")
require('pkgVerificationPassed=' in install_handoff, "install-handoff-missing-pkg-verification")
require('repairTISDuplicatesRequired=' in install_handoff, "install-handoff-missing-duplicate-repair-summary")
require('INPUTIA_REPAIR_TIS_DUPLICATES=1 ./repair-tis-duplicates.sh' in install_handoff, "install-handoff-missing-duplicate-repair-command")
require('handoffOpensGUI=false' in install_handoff, "install-handoff-missing-no-gui-marker")
require('handoffChangesSystemInputSource=false' in install_handoff, "install-handoff-missing-no-input-source-marker")
require('installCheckBlockReasons=none' in install_handoff, "install-handoff-missing-success-criteria")
require('installCheckTISDuplicateMatches=false' in install_handoff, "install-handoff-missing-duplicate-success-criteria")
require('installCheckRequiredActions=none' in install_handoff, "install-handoff-missing-actions-success-criteria")
for forbidden in [
    "menu-readiness.sh",
    "gui-smoke-readiness.sh",
    "gui-smoke-suite.sh",
    "INPUTIA_RUN_UI_SMOKE=1",
    "notarization-readiness.sh",
]:
    require(forbidden not in install_check, f"install-check-unexpected-{forbidden}")

require('validationTier=tis-duplicate-repair' in repair_tis_duplicates, "repair-tis-missing-tier-marker")
require('touchesMenuBar=false' in repair_tis_duplicates, "repair-tis-missing-menu-policy")
require('opensGUI=false' in repair_tis_duplicates, "repair-tis-missing-gui-policy")
require('INPUTIA_REPAIR_TIS_DUPLICATES:-0' in repair_tis_duplicates, "repair-tis-missing-opt-in-gate")
require('tisDuplicateRepairReady=false reason=opt-in-required' in repair_tis_duplicates, "repair-tis-missing-default-deny")
require('INPUTIA_REPAIR_TIS_READINESS_FOR_TEST' in repair_tis_duplicates, "repair-tis-missing-offline-self-check-readiness")
require('INPUTIA_REPAIR_SYSTEM_MATCHES_BUILD_FOR_TEST' in repair_tis_duplicates, "repair-tis-missing-system-match-self-check")
require('tisDuplicateRepairReady=false reason=system-cdhash-mismatch' in repair_tis_duplicates, "repair-tis-missing-system-match-gate")
require('run-install-handoff-and-admin-install-before-repair' in repair_tis_duplicates, "repair-tis-missing-install-before-repair-action")
require('--disable-all-inputia-sources' in repair_tis_duplicates, "repair-tis-missing-disable-all-command")
require('--register-input-source' in repair_tis_duplicates, "repair-tis-missing-register-command")
require('--enable-input-source' in repair_tis_duplicates, "repair-tis-missing-enable-command")
require('--select-input-source' in repair_tis_duplicates, "repair-tis-missing-select-command")
require('INPUTIA_REPAIR_TIS_DUPLICATES_SELF_CHECK' in repair_tis_duplicates, "repair-tis-missing-self-check")
require('repair-tis-duplicates.sh' not in dev_fast, "dev-fast-unexpected-repair-tis")
require('"$ROOT_DIR/repair-tis-duplicates.sh"' not in install_check, "install-check-unexpected-executed-repair-tis")

require('InputiaCandidatePanelFormatter' in candidate_panel, "candidate-panel-missing-formatter")
require('maximumCandidateCount = 9' in candidate_panel, "candidate-panel-missing-nine-candidate-cap")
require('expanded ? "\\n" : "   "' in candidate_panel, "candidate-panel-missing-expanded-line-break")
require('InputiaCandidatePanelFormatter.candidateString' in candidate_panel, "candidate-panel-show-not-using-formatter")
require('InputiaCandidatePanelSelfCheck.swift' in build, "build-missing-candidate-panel-self-check-tool")
require('inputia-candidate-panel-self-check' in build, "build-missing-candidate-panel-self-check-output")
require('InputiaExpectedHostCDHash' in build, "build-missing-settings-launcher-expected-host-cdhash")
require('settingsLauncherBuildSelfCheck=true' in settings_launcher_self_check, "settings-launcher-self-check-missing-pass-marker")
require('expected-host-cdhash-mismatch' in settings_launcher_self_check, "settings-launcher-self-check-missing-cdhash-mismatch")
require('InputiaExpectedHostCDHash' in settings_launcher_self_check, "settings-launcher-self-check-missing-expected-host-cdhash")
require('bundleCDHash(at:' in read("SettingsLauncher/main.swift"), "settings-launcher-source-missing-cdhash-check")
require('build-settings-expected-host-cdhash-mismatch' in verify_pkg, "verify-pkg-missing-build-settings-host-cdhash-check")
require('archive-settings-expected-host-cdhash-mismatch' in verify_pkg, "verify-pkg-missing-archive-settings-host-cdhash-check")
require('archiveSettingsExpectedHostCDHash=' in verify_pkg, "verify-pkg-missing-archive-settings-host-cdhash-output")
require('buildSettingsExpectedHostCDHash=' in verify_pkg, "verify-pkg-missing-build-settings-host-cdhash-output")
require('candidatePanelSelfCheck=' in candidate_panel_self_check, "candidate-panel-self-check-missing-pass-marker")
require('candidatePanelExpandedBreaksLines' in candidate_panel_self_check, "candidate-panel-self-check-missing-expanded-lines-case")
require('candidatePanelCollapsedContainsSeventhCandidate' in candidate_panel_self_check, "candidate-panel-self-check-missing-seventh-candidate-case")
require('candidatePanelExpandedCapsAtNineCandidates' in candidate_panel_self_check, "candidate-panel-self-check-missing-cap-case")

require('validationTier=release/full-check' in full_check, "full-check-missing-tier-marker")
require('touchesMenuBar=true' in full_check, "full-check-missing-menu-policy")
require('opensGUI=true' in full_check, "full-check-missing-gui-policy")
require('checksNotarization=true' in full_check, "full-check-missing-notarization-policy")
require('INPUTIA_MENU_READINESS_CACHE_FILE="$MENU_CACHE"' in full_check, "full-check-missing-menu-cache")
require('INPUTIA_MENU_READINESS_ALLOW_AXPRESS=1' in full_check, "full-check-missing-menu-opt-in")
require('INPUTIA_GUI_SMOKE_READINESS_ALLOW_CHECK=1' in full_check, "full-check-missing-gui-readiness-opt-in")
require('INPUTIA_RUN_UI_SMOKE=1' in full_check, "full-check-missing-ui-smoke-opt-in")
require('"$ROOT_DIR/menu-readiness.sh"' in full_check, "full-check-missing-menu-readiness")
require('"$ROOT_DIR/post-install-regression.sh" "$APP"' in full_check, "full-check-missing-postinstall-regression")
require('"$ROOT_DIR/notarization-readiness.sh"' in full_check, "full-check-missing-notarization-readiness")

require('INPUTIA_MENU_READINESS_ALLOW_AXPRESS:-0' in menu_readiness, "menu-readiness-missing-axpress-opt-in")
require('menuAXPressAllowed=false' in menu_readiness, "menu-readiness-missing-deny-marker")
require('menuReadinessBlockReason=opt-in-required' in menu_readiness, "menu-readiness-missing-opt-in-reason")
require('menuReadinessCache=hit' in menu_readiness, "menu-readiness-missing-cache-hit")
require(menu_readiness.index('menuReadinessCache=hit') < menu_readiness.index('/usr/bin/osascript'), "menu-readiness-cache-after-osascript")
require(menu_readiness.index('INPUTIA_MENU_READINESS_ALLOW_AXPRESS') < menu_readiness.index('/usr/bin/osascript'), "menu-readiness-gate-after-osascript")

require('INPUTIA_MENU_READINESS_ALLOW_AXPRESS=1 "$ROOT_DIR/menu-readiness.sh"' in status, "status-menu-opt-in-not-explicit")
require('INPUTIA_STATUS_INCLUDE_MENU_READINESS:-0' in status, "status-missing-menu-include-gate")
require('INPUTIA_STATUS_INCLUDE_GUI_SMOKE_READINESS:-0' in status, "status-missing-gui-readiness-gate")
require('statusGuiSmokeReady=unknown reason=skipped' in status, "status-default-gui-summary-not-skipped")
require('app_expected_host_cdhash' in status, "status-missing-settings-expected-host-helper")
require('targetSettingsExpectedHostCDHash=' in status, "status-missing-target-settings-expected-host-cdhash")
require('targetSettingsMatchesBuild=' in status, "status-missing-target-settings-build-match")

require('INPUTIA_MENU_READINESS_ALLOW_AXPRESS=1 "$ROOT_DIR/menu-readiness.sh"' in tis, "tis-menu-opt-in-not-explicit")
require('INPUTIA_TIS_INCLUDE_MENU_READINESS:-0' in tis, "tis-missing-menu-include-gate")

require('INPUTIA_RUN_UI_SMOKE:-0' in post_install, "post-install-missing-ui-smoke-gate")
require('uiSmokeSkipped=true reason=disabled' in post_install, "post-install-missing-default-skip-marker")

require('rimeLatencyTouchesMenuBar=false' in rime_latency, "rime-latency-missing-menu-policy")
require('rimeLatencyOpensGUI=false' in rime_latency, "rime-latency-missing-gui-policy")
require('rimeLatencyChangesSystemInputSource=false' in rime_latency, "rime-latency-missing-input-source-policy")
require('rimeLatencyChecksNotarization=false' in rime_latency, "rime-latency-missing-notarization-policy")
require('persistent_session_probe' in rime_latency, "rime-latency-missing-persistent-probe")
require('rimeLatencySelfCheck=true' in rime_latency, "rime-latency-missing-pass-marker")
require('INPUTIA_RIME_LATENCY_MAX_INCREMENTAL_MS' in rime_latency, "rime-latency-missing-threshold-override")
require('INPUTIA_RIME_LATENCY_MIN_SPEEDUP' in rime_latency, "rime-latency-missing-speedup-override")

require('INPUTIA_GUI_SMOKE_READINESS_ALLOW_CHECK:-0' in gui_readiness, "gui-readiness-missing-opt-in")
require('guiSmokeReadinessOptInRequired=true' in gui_readiness, "gui-readiness-missing-opt-in-marker")
require('guiSmokeReadinessReady=false reason=opt-in-required' in gui_readiness, "gui-readiness-missing-opt-in-ready-line")
require('settings.buildExpectedHostCDHash=' in gui_readiness, "gui-readiness-missing-build-settings-host-cdhash")
require('settings.targetExpectedHostCDHash=' in gui_readiness, "gui-readiness-missing-target-settings-host-cdhash")
require('target_settings_expected_host_cdhash" == "$build_cdhash"' in gui_readiness, "gui-readiness-settings-match-not-bound-to-host-cdhash")

require('INPUTIA_RUN_UI_SMOKE:-0' in gui_suite, "gui-suite-missing-ui-smoke-gate")
require('INPUTIA_GUI_SMOKE_READINESS_ALLOW_CHECK=1' in gui_suite, "gui-suite-missing-readiness-opt-in")
require('guiSmokeSuiteReady=false reason=ui-smoke-opt-in-required' in gui_suite, "gui-suite-missing-opt-in-marker")

print("validationPolicySelfCheck=true")
PY
