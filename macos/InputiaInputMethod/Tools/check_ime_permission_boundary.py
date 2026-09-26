#!/usr/bin/env python3
"""Reject privileged cross-app APIs in the IME, including linked symbols."""
import argparse
from pathlib import Path
import re
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('--binary', type=Path)
args = parser.parse_args()
root = Path(__file__).resolve().parents[1]
source_api = re.compile(r'AXIsProcessTrusted|AXUIElement|AXObserver|addGlobalMonitorForEvents|CGEventSource[.]keyState|CGWindowListCopyWindowInfo')
for path in (root/'Sources/InputiaInputMethod').glob('*.swift'):
    if source_api.search(path.read_text()):
        raise SystemExit(f'IME privileged API in {path.name}')
main = (root/'Sources/InputiaInputMethod/main.swift').read_text()
retirement = main.split('func synchronizePermissionEpoch()', 1)[1].split('private let voiceControllerID', 1)[0]
if 'clearSharedChineseCandidates()' not in retirement or 'sharedChineseOrder = nil' in retirement or 'clearInputState(' in retirement:
    raise SystemExit('IME permission retirement must restore original candidate order without clearing composition')
reuse = main.split('func shortcutRegistrationTarget()', 1)[1].split('func acceptUnifiedShortcut', 1)[0]
if not all(guard in reuse for guard in ['isCurrentForShortcut(', 'localCompositionGeneration', 'localSelectionGeneration', 'discardPreparedVoiceTarget(']):
    raise SystemExit('IME shortcut reuse must validate local target and release obsolete broker grant')
if args.binary:
    symbols = subprocess.check_output(['/usr/bin/nm', '-u', str(args.binary)], text=True)
    if re.search(r'AXIsProcessTrusted|AXUIElement|AXObserver|CGEventSourceKeyState|CGWindowListCopyWindowInfo', symbols):
        raise SystemExit('IME links a privileged API')
    strings = subprocess.check_output(['/usr/bin/strings', str(args.binary)], text=True)
    if 'addGlobalMonitorForEventsMatchingMask' in strings:
        raise SystemExit('IME includes a global event monitor selector')
print('imePermissionBoundary=true localAX=false globalKeyMonitoring=false')
