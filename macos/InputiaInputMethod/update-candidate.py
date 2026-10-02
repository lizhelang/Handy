#!/usr/bin/env python3
"""受控更新正式两组件：固定身份、维护屏障、旧名称迁移和事务回滚；不读写TCC授权。"""
import argparse
import fcntl
import sqlite3
import json
import os
from pathlib import Path
import plistlib
import re
import shutil
import signal
import subprocess
import tempfile
import time
import uuid

ROOT = Path(__file__).resolve().parent
REPO = ROOT.parent.parent
REGISTRAR = '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister'


def run(*args, timeout=30):
    return subprocess.check_output([str(a) for a in args], stderr=subprocess.STDOUT, text=True, timeout=timeout)



def start_registered_components(apps):
    # 替换 app 后先刷新注册，否则 TIS 显示选中但 IMK 会拒绝旧缓存的连接名。
    # 注册失败时停止启动，交给外层事务回滚；不修改输入法身份或系统授权。
    registrar = '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister'
    for app in apps:
        run(registrar, '-f', app)
    run('/usr/bin/open', '-a', apps[0], '--args', '--start-hidden')
    run('/usr/bin/open', '-a', apps[1])


def canonical(path):
    path = Path(path).absolute()
    if path.resolve() != path or path.is_symlink():
        raise ValueError(f"拒绝符号链接或非标准路径: {path}")
    return path


def control_installation(applications=Path('/Applications')):
    current = canonical(applications/'Inputia.app')
    legacy = canonical(applications/'Inputia Candidate.app')
    if current.exists() and legacy.exists():
        raise ValueError('正式与旧名称安装同时存在；拒绝覆盖或选择任一版本')
    if current.is_dir():
        return current, current
    if legacy.is_dir() and not current.exists():
        return legacy, current
    raise ValueError('未找到唯一可更新的 Inputia 安装')


def settings_installation(applications=Path('/Applications')):
    """定位唯一设置启动器；v2 必须与主程序、输入法一起换代。"""
    system = canonical(applications/'Inputia 设置.app')
    user = canonical(Path.home()/'Applications'/'Inputia 设置.app')
    existing = [path for path in (system, user) if path.exists()]
    if len(existing) > 1:
        raise ValueError('系统与用户目录同时存在设置启动器；拒绝选择其一覆盖')
    if not existing:
        raise ValueError('未找到已安装的 Inputia 设置.app；v2 不执行缺组件安装')
    return existing[0]


def unregister_legacy_control(old_control, new_control, old_backup=None):
    if old_control == new_control:
        return True
    try:
        # 原 URL 已随事务迁移消失，使用保留旧签名包的有效 URL 清理其登记。
        target = canonical(old_backup if old_backup is not None else old_control)
        if target == canonical(new_control) or not target.is_dir():
            raise ValueError('旧登记清理路径无效；拒绝注销新安装')
        run(REGISTRAR, '-u', target)
        return True
    except (subprocess.SubprocessError, OSError, ValueError):
        # 已验证的新安装保持运行；清理旧登记失败不应误触发回滚或第二次安装。
        print('legacyRegistrationRemoved=false releaseInstalled=true cleanupPending=true', flush=True)
        return False


def validate_profile(app, role, run_id):
    prefix = 'Handy' if role == 'control' else 'Inputia'
    expected_id = 'com.pais.handy.UnifiedCandidate' if role == 'control' else 'com.inputia.inputmethod.Inputia.UnifiedCandidate'
    with (app/'Contents/Info.plist').open('rb') as stream:
        info = plistlib.load(stream)
    if (info.get('CFBundleIdentifier') != expected_id
            or info.get(prefix+'DevelopmentCandidate') is not True
            or info.get(prefix+'ProfileRunID') != run_id):
        raise ValueError('组件身份或数据 profile 不匹配；拒绝迁移')


def validate_release_v2(app, role, release_id):
    expected_id = {
        'control': 'com.pais.handy.UnifiedCandidate',
        'ime': 'com.inputia.inputmethod.Inputia.UnifiedCandidate',
        'settings': 'com.inputia.settings.UnifiedCandidate',
    }[role]
    with (app/'Contents/Info.plist').open('rb') as stream:
        info = plistlib.load(stream)
    if (info.get('CFBundleIdentifier') != expected_id
            or info.get('InputiaReleaseID') != release_id
            or any(key in info for key in ('HandyProfileRunID', 'HandyDevelopmentCandidate', 'InputiaProfileRunID', 'InputiaDevelopmentCandidate'))):
        raise ValueError('组件 release 身份或 v1 开发标记不匹配；拒绝 v2 迁移')


def atomic_json(path, data):
    fd, temporary = tempfile.mkstemp(prefix='.permission-update-', dir=path.parent)
    try:
        with os.fdopen(fd, 'w') as stream:
            json.dump(data, stream); stream.flush(); os.fsync(stream.fileno())
        os.replace(temporary, path)
        directory_fd = os.open(path.parent, os.O_RDONLY | getattr(os, 'O_DIRECTORY', 0))
        try:
            os.fsync(directory_fd)
        finally:
            os.close(directory_fd)
    finally:
        if os.path.exists(temporary): os.unlink(temporary)


def write_legacy_receipt(run_id, release_id, destinations, home=None):
    """在 v2 提交点登记旧 profile；收据不改变数据路径，也不授权系统目录操作。"""
    home = Path.home() if home is None else Path(home)
    support = home / 'Library/Application Support/Inputia'
    support.mkdir(mode=0o700, parents=True, exist_ok=True)
    receipt_path = support / 'installation.json'
    installation_id = str(uuid.uuid4())
    if receipt_path.exists():
        with receipt_path.open('r', encoding='utf-8') as stream:
            previous = json.load(stream)
        if (previous.get('schema_version') != 1
                or previous.get('product_id') != 'com.inputia'
                or previous.get('scope') != 'legacy_single_user'
                or previous.get('profile_id') != f'unified-candidate:{run_id}'
                or not re.fullmatch(r'[0-9a-f-]{36}', str(previous.get('installation_id', '')))):
            raise ValueError('已有安装收据身份或作用域不匹配；拒绝覆盖')
        installation_id = previous['installation_id']
    receipt = {
        'schema_version': 1,
        'product_id': 'com.inputia',
        'installation_id': installation_id,
        'profile_id': f'unified-candidate:{run_id}',
        'uid': os.getuid(),
        'scope': 'legacy_single_user',
        'data': {'kind': 'legacy_candidate', 'run_id': run_id},
        'components': {
            'control': str(destinations[0]),
            'ime': str(destinations[1]),
            'settings': str(destinations[2]),
        },
        'release_id': release_id,
        'channel': 'candidate',
    }
    atomic_json(receipt_path, receipt)
    return receipt_path


def validate_existing_legacy_receipt(run_id, destinations, home=None):
    """只读核对已有收据；缺失表示首次登记，不把缺失伪装成已验证。"""
    home = Path.home() if home is None else Path(home)
    path = home / 'Library/Application Support/Inputia/installation.json'
    if not path.exists():
        return False
    stat_result = path.stat()
    if stat_result.st_uid != os.getuid() or stat_result.st_mode & 0o077:
        raise ValueError('已有安装收据所有者或权限不安全')
    with path.open('r', encoding='utf-8') as stream:
        value = json.load(stream)
    expected_components = {
        'control': str(destinations[0]),
        'ime': str(destinations[1]),
        'settings': str(destinations[2]),
    }
    if (value.get('schema_version') != 1
            or value.get('product_id') != 'com.inputia'
            or value.get('scope') != 'legacy_single_user'
            or value.get('profile_id') != f'unified-candidate:{run_id}'
            or value.get('uid') != os.getuid()
            or value.get('channel') != 'candidate'
            or not re.fullmatch(r'inputia-[A-Za-z0-9._-]{1,180}', str(value.get('release_id', '')))
            or value.get('data') != {'kind': 'legacy_candidate', 'run_id': run_id}
            or value.get('components') != expected_components
            or not re.fullmatch(r'[0-9a-f-]{36}', str(value.get('installation_id', '')))):
        raise ValueError('已有安装收据与当前 legacy 组件或数据 profile 不匹配')
    return True


def identity(path):
    text = run('/usr/bin/codesign', '-d', '-r-', path)
    requirement = next(line.split('designated => ', 1)[1] for line in text.splitlines() if 'designated => ' in line)
    if 'certificate' not in requirement or 'cdhash' in requirement:
        raise ValueError('更新必须保留稳定的证书身份，拒绝临时哈希签名')
    run('/usr/bin/codesign', '--verify', '--deep', '--strict', path)
    return requirement


def process_ids(apps):
    executables = set()
    for app in apps:
        with (app/'Contents/Info.plist').open('rb') as f: info = plistlib.load(f)
        executables.add(str(app/'Contents/MacOS'/info['CFBundleExecutable']))
    found = []
    for line in run('/bin/ps', '-axo', 'pid=,comm=').splitlines():
        bits = line.strip().split(None, 1)
        if len(bits) == 2 and bits[1] in executables: found.append(int(bits[0]))
    return found


def stop_known(apps):
    # 每个PID须仍运行与当前安装文件相同的映像，才发送终止信号。
    for app in apps:
        for pid in process_ids([app]):
            run('/bin/bash', ROOT/'install-check.sh', '--running-identity', app, pid)
            os.kill(pid, signal.SIGTERM)
    deadline = time.monotonic()+5
    while process_ids(apps):
        if time.monotonic() >= deadline:
            raise RuntimeError('组件尚未退出；停止更新，保留备份，不覆盖运行中的程序')
        time.sleep(.1)


def maintenance_ready(profile, live_pids, marker_epoch=None, now=None):
    now = time.time() if now is None else now
    covered = set()
    live = set(live_pids)
    for name in ['permission-health-background.json', 'permission-health-ime.json']:
        try:
            data = json.loads((profile/name).read_text())
            # 已退出的组件不再提供新回执；仍运行的每个 PID 必须完整确认。
            if data['pid'] not in live: continue
            if now - data['updated_at_ms']/1000 > 5 or data['updated_at_ms']/1000 > now+1: return False
            if marker_epoch is not None and data.get('maintenance_marker_epoch') != marker_epoch and data.get('marker_epoch') != marker_epoch:
                return False
            if data.get('state') not in ['maintenance', 'stopped'] and data.get('maintenance_state') != 'active': return False
            covered.add(data['pid'])
        except (OSError, ValueError, KeyError, TypeError): continue
    return live.issubset(covered)


def install_transaction(destinations, staged, backups, pair_path, new_pair, old_pair, originals=None):
    originals = destinations if originals is None else originals
    moved, installed = [], []
    try:
        for original, dst in zip(originals, destinations):
            if original != dst and dst.exists():
                raise ValueError('迁移目标已存在；拒绝覆盖')
        for original, dst, src, backup in zip(originals, destinations, staged, backups):
            original.rename(backup); moved.append((original, backup))
            src.rename(dst); installed.append(dst)
        shutil.copy2(new_pair, pair_path.with_suffix('.update'))
        os.replace(pair_path.with_suffix('.update'), pair_path)
    except Exception:
        for path in installed: shutil.rmtree(path)
        for dst, backup in reversed(moved): backup.rename(dst)
        shutil.copy2(old_pair, pair_path)
        raise


def rollback_installation(destinations, originals, backups, failed_root, pair_path, old_pair):
    # 失败的新包留在备份域；原包回到原路径，绝不保留第二个可启动安装。
    for index, (target, original, backup) in enumerate(zip(destinations, originals, backups)):
        if target.exists():
            target.rename(failed_root/f'component-{index}-failed.app')
        if original.exists():
            raise ValueError('回滚原路径被其他程序占用；拒绝覆盖')
        shutil.copytree(backup, original, symlinks=True)
    shutil.copy2(old_pair, pair_path)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--run-id', required=True)
    parser.add_argument('--control-app', required=True)
    parser.add_argument('--inputia-app', required=True)
    parser.add_argument('--settings-app', help='v2 设置启动器；v1 路径不使用')
    parser.add_argument('--pair-manifest', required=True)
    parser.add_argument('--public-build', required=True)
    parser.add_argument('--build-context', help='v2 public-build 对应的 metadata/build-context.json')
    parser.add_argument('--release-v2', action='store_true', help='新组件使用 releaseId 绑定的 v2 身份；目标数据域仍由 --run-id 指定')
    parser.add_argument('--apply', action='store_true', help='默认只校验；此开关才执行更新')
    args = parser.parse_args()
    if not re.fullmatch(r'[A-Za-z0-9_-]{1,64}', args.run_id): raise ValueError('invalid run ID')
    profile = canonical(Path.home()/'Library/Application Support/HandyUnifiedCandidate'/args.run_id)
    pair = profile/'pair-manifest.json'
    # 同一profile不允许两个更新器交错替换包或配对清单。
    lock_fd = os.open(profile/'.candidate-update.lock', os.O_WRONLY | os.O_CREAT | os.O_NOFOLLOW, 0o600)
    try:
        fcntl.flock(lock_fd, fcntl.LOCK_EX | fcntl.LOCK_NB)
    except BlockingIOError:
        os.close(lock_fd)
        raise RuntimeError('另一个候选更新正在执行')
    old_control, new_control = control_installation()
    ime = canonical(Path.home()/'Library/Input Methods/InputiaUnifiedCandidate.app')
    old_settings = settings_installation() if args.release_v2 else None
    new_settings = canonical(args.settings_app) if args.release_v2 and args.settings_app else None
    if args.release_v2 and new_settings is None:
        raise ValueError('v2 更新缺少 --settings-app；拒绝留下旧设置组件')
    originals = [old_control, ime] + ([old_settings] if old_settings else [])
    destinations = [new_control, ime] + ([old_settings] if old_settings else [])
    sources = [canonical(args.control_app), canonical(args.inputia_app)] + ([new_settings] if new_settings else [])
    receipt_present = validate_existing_legacy_receipt(args.run_id, originals) if args.release_v2 else False
    if args.release_v2:
        print(f'installationReceiptPresent={str(receipt_present).lower()}', flush=True)
    metadata = canonical(args.public_build); manifest = canonical(args.pair_manifest)
    if args.release_v2:
        public = json.loads(metadata.read_text())
        if public.get('schema_version') != 2 or public.get('product_id') != 'com.inputia' or not isinstance(public.get('release_id'), str):
            raise ValueError('无效 v2 public-build 元数据')
        if not args.build_context:
            raise ValueError('v2 更新缺少 build context')
        release_id = public['release_id']
    else:
        release_id = None
    roles = ['control', 'ime'] + (['settings'] if args.release_v2 else [])
    for role, old, new in zip(roles, originals, sources):
        if new in originals or new in destinations: raise ValueError('构建源不能是安装路径')
        if role != 'settings':
            validate_profile(old, role, args.run_id)
        else:
            # 旧设置启动器可能是历史 ad-hoc 身份；只确认它是完整 bundle，
            # 新入口必须通过严格签名和 v2 release 身份检查。
            run('/usr/bin/codesign', '--verify', '--deep', '--strict', old)
        validate_release_v2(new, role, release_id) if args.release_v2 else validate_profile(new, role, args.run_id)
        if role != 'settings' and identity(old) != identity(new):
            raise ValueError('更新签名身份改变；拒绝要求用户反复重新授权')
        if role == 'settings':
            run('/usr/bin/codesign', '--verify', '--deep', '--strict', new)
    # 验证编译期公开元数据的归属/权限/内容；不使用清单本身提供的新信任根。
    import importlib.util
    spec = importlib.util.spec_from_file_location('build_trust', REPO/'native/unified-pair-auth/build_trust.py')
    trust_module = importlib.util.module_from_spec(spec); spec.loader.exec_module(trust_module)
    if args.release_v2:
        trust_module.load(metadata, context_path=canonical(Path(args.build_context)))
    else:
        trust_module.load(metadata, args.run_id)
    with tempfile.TemporaryDirectory(prefix='inputia-update-verify-') as temporary:
        verifier = Path(temporary)/'verify'
        if args.release_v2:
            run('/usr/bin/swiftc', '-parse-as-library', REPO/'native/unified-pair-auth/UnifiedPairAuth.swift', REPO/'native/unified-pair-auth/ReleasePairAuthVerify.swift', '-o', verifier, timeout=90)
            print(run(verifier, metadata, metadata, manifest, *sources[:2]).strip())
        else:
            run('/usr/bin/swiftc', '-parse-as-library', REPO/'native/unified-pair-auth/UnifiedPairAuth.swift', ROOT/'Tools/CandidateUpdateVerify.swift', '-o', verifier, timeout=90)
            print(run(verifier, metadata, pair, pair, *originals, args.run_id).strip())
            print(run(verifier, metadata, pair, manifest, *sources, args.run_id).strip())
        if not args.apply:
            print('updatePreflight=true permissionRecordsUnchanged=true'); return
        db_path = profile/'Handy/integration.db'
        with sqlite3.connect(db_path.as_uri()+'?mode=ro', uri=True) as db:
            active = db.execute("select count(*) from unified_voice_sessions where retired=0 and json_extract(view_json,'$.phase') in ('preparing','recording','processing')").fetchone()[0]
        if active:
            raise RuntimeError('语音会话正在进行，未更新程序或改变维护状态')
        backup_base = Path.home()/'Library/Application Support/HandyUnifiedBuilds'
        backup_base.mkdir(mode=0o700, parents=True, exist_ok=True)
        backup = Path(tempfile.mkdtemp(prefix='permission-update-', dir=backup_base))
        staged_names = ['control-staged.app', 'ime-staged.app'] + (['settings-staged.app'] if args.release_v2 else [])
        staged = [backup/name for name in staged_names]
        for src, dst in zip(sources, staged): shutil.copytree(src, dst, symlinks=True)
        shutil.copy2(pair, backup/'pair-before.json')
        shutil.copy2(manifest, backup/'pair-new.json')
        # v2 配对清单只包含 control/IME 两个协议 peer；设置启动器仍做独立
        # codesign 校验，不能把它和 run_id 误传给只接受五个业务参数的验签器。
        print(run(verifier, metadata, metadata, backup/'pair-new.json', *staged[:2]).strip())
        marker = profile/'permission-maintenance.json'
        token = str(uuid.uuid4())
        atomic_json(marker, {'schema_version':1, 'active':True, 'epoch':token})
        print(f'updateBackup={backup}', flush=True)
        with (destinations[1]/'Contents/Info.plist').open('rb') as f: old_version = int(plistlib.load(f)['CFBundleVersion'])
        pids = process_ids(originals)
        if old_version >= 67 and pids:
            deadline = time.monotonic()+10
            while not maintenance_ready(profile, pids, token):
                if time.monotonic() >= deadline: raise RuntimeError('维护屏障未确认，未修改程序；保持暂停以便诊断')
                time.sleep(.2)
        # 首次从66升级时尚无维护协议，但仍必须先切离且确认所有旧进程退出。
        tis = Path(temporary)/'tis'
        run('/usr/bin/swiftc', '-parse-as-library', ROOT/'Tools/InputiaTISTool.swift', '-o', tis)
        current = run(tis, '--dump-current-input-source')
        previous_source = next(line[3:] for line in current.splitlines() if line.startswith('id='))
        switched = run(tis, '--select-source-id', 'com.apple.keylayout.ABC')
        if 'selectCurrentMatchesTarget=true' not in switched: raise RuntimeError('未确认切离输入法，停止更新')
        stop_known(originals)
        backup_names = ['control-before.app', 'ime-before.app'] + (['settings-before.app'] if args.release_v2 else [])
        backups = [backup/name for name in backup_names]
        install_transaction(destinations, staged, backups, pair, backup/'pair-new.json', backup/'pair-before.json', originals)
        try:
            run(verifier, metadata, backup/'pair-before.json', pair, *destinations[:2], args.run_id)
            atomic_json(marker, {'schema_version':1, 'active':False, 'epoch':str(uuid.uuid4())})
            start_registered_components(destinations)
            deadline = time.monotonic()+10
            while len(process_ids(destinations[:2])) < 2:
                if time.monotonic() >= deadline: raise RuntimeError('新组件未启动')
                time.sleep(.2)
            for app in destinations[:2]:
                pids = process_ids([app])
                if len(pids) != 1: raise RuntimeError('检测到重复组件进程')
                print(run('/bin/bash', ROOT/'install-check.sh', '--running-identity', app, pids[0]).strip())
            # 恢复原输入源；没有权限的语音功能仍由后端闭门，不更改系统授权。
            restored = run(tis, '--select-source-id', previous_source)
            if 'selectCurrentMatchesTarget=true' not in restored:
                raise RuntimeError('原输入源恢复未得到确认')
            print(restored.strip())
            receipt_path = write_legacy_receipt(args.run_id, release_id, destinations)
            print(f'installationReceipt={receipt_path}', flush=True)
        except Exception:
            atomic_json(marker, {'schema_version':1, 'active':True, 'epoch':str(uuid.uuid4())})
            stop_known(destinations)
            rollback_installation(destinations, originals, backups, backup, pair, backup/'pair-before.json')
            raise
        if old_control != new_control:
            # 新路径已启动且动态身份核验通过，才清理旧 LaunchServices 登记。
            unregister_legacy_control(old_control, new_control, backups[0])
        print('releaseUpdate=true tccChanged=false previousRecordingsReplayed=false')


if __name__ == '__main__':
    main()
