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
full_check = read("release/full-check.sh")
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

require('validationTier=install-check' in install_check, "install-check-missing-tier-marker")
require('INPUTIA_TIS_INCLUDE_MENU_READINESS=0 "$ROOT_DIR/tis-readiness.sh"' in install_check, "install-check-must-disable-menu-readiness")
require('installCheckBlockReasons=' in install_check, "install-check-missing-block-reasons")
require('installCheckRequiredAction=' in install_check, "install-check-missing-required-action")
require('INPUTIA_INSTALL_CHECK_SELF_CHECK:-0' in install_check, "install-check-missing-self-check-mode")
require('installCheckSelfCheck=true' in install_check, "install-check-missing-self-check-pass-marker")
require('run-install-handoff-and-admin-install' in install_check, "install-check-missing-admin-required-action")
require('restart-inputia-host-after-install' in install_check, "install-check-missing-running-host-action")
require('tis-duplicate-matches' in install_check, "install-check-missing-tis-duplicate-blocker")
require('remove-duplicate-inputia-and-readd-once' in install_check, "install-check-missing-tis-duplicate-action")
require('"$ROOT_DIR/install-check.sh"' in dev_fast, "dev-fast-missing-install-check-self-check")
require('INPUTIA_INSTALL_CHECK_SELF_CHECK=1 "$ROOT_DIR/install-check.sh"' in dev_fast, "dev-fast-install-check-not-self-check-only")
require('installCheckBlockReasons=' in install_handoff, "install-handoff-missing-block-reasons")
require('installCheckRequiredAction=' in install_handoff, "install-handoff-missing-required-action")
require('installCheckPassed=' in install_handoff, "install-handoff-missing-install-check-pass")
require('handoffOpensGUI=false' in install_handoff, "install-handoff-missing-no-gui-marker")
require('handoffChangesSystemInputSource=false' in install_handoff, "install-handoff-missing-no-input-source-marker")
require('installCheckBlockReasons=none' in install_handoff, "install-handoff-missing-success-criteria")
for forbidden in [
    "menu-readiness.sh",
    "gui-smoke-readiness.sh",
    "gui-smoke-suite.sh",
    "INPUTIA_RUN_UI_SMOKE=1",
    "notarization-readiness.sh",
]:
    require(forbidden not in install_check, f"install-check-unexpected-{forbidden}")

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

require('INPUTIA_RUN_UI_SMOKE:-0' in gui_suite, "gui-suite-missing-ui-smoke-gate")
require('INPUTIA_GUI_SMOKE_READINESS_ALLOW_CHECK=1' in gui_suite, "gui-suite-missing-readiness-opt-in")
require('guiSmokeSuiteReady=false reason=ui-smoke-opt-in-required' in gui_suite, "gui-suite-missing-opt-in-marker")

print("validationPolicySelfCheck=true")
PY
