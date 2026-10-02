#!/usr/bin/env python3
"""Inputia 发布元数据工具。结构验证、文件摘要和发布信任分别报告。"""
from __future__ import annotations

import argparse
import base64
import binascii
import copy
import datetime as dt
import hashlib
import json
import os
from pathlib import Path, PurePosixPath
import platform
import plistlib
import re
import stat
import subprocess
import sys
import tempfile
import uuid

if sys.version_info < (3, 11):
    raise SystemExit("Inputia 发布工具需要 Python >= 3.11；请设置 INPUTIA_RELEASE_PYTHON。系统 Python 3.9 不支持 TOML。")
import tomllib

ROOT = Path(__file__).resolve().parent.parent
SCHEMAS = ROOT / "release/schema"
PRODUCT = ROOT / "release/product.toml"
MAX_DOCUMENT = 4 * 1024 * 1024
HEX_DIGEST = re.compile(r"^[0-9a-f]{64}$")


class ReleaseError(ValueError):
    """可向开发者显示且不包含凭据的发布合同错误。"""


def require(condition, message):
    if not condition:
        raise ReleaseError(message)


def unique_pairs(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, f"重复 JSON 字段：{key}")
        result[key] = value
    return result


def read_json(path):
    with Path(path).open("rb") as stream:
        raw = stream.read(MAX_DOCUMENT + 1)
    require(len(raw) <= MAX_DOCUMENT, "JSON 文档过大")
    try:
        return json.loads(raw, object_pairs_hook=unique_pairs,
                          parse_constant=lambda value: (_ for _ in ()).throw(ReleaseError(f"非法 JSON 数字：{value}")))
    except (UnicodeDecodeError, json.JSONDecodeError) as error:
        raise ReleaseError("无法解析 UTF-8 JSON 文档") from error


def canonical_bytes(value):
    # 当前合同仅允许有界整数；未来添加浮点字段前须升级编码合同。
    return json.dumps(value, ensure_ascii=False, sort_keys=True, separators=(",", ":"), allow_nan=False).encode("utf-8")


def validate_schema(value, schema, document=None, path="$", *, strict_schema=True):
    """执行仓库 schema 使用的有界子集；遇到未实现关键字就拒绝。"""
    document = document or schema
    known = {"$schema", "$id", "$defs", "$ref", "type", "const", "enum", "properties", "required", "additionalProperties", "items", "minItems", "maxItems", "uniqueItems", "minimum", "maximum", "minLength", "maxLength", "pattern", "description", "title"}
    if strict_schema:
        require(set(schema) <= known, f"{path}: schema 含不支持的关键字")
    if "$ref" in schema:
        ref = schema["$ref"]
        require(ref.startswith("#/$defs/"), "只允许文档内 schema 引用")
        return validate_schema(value, document["$defs"][ref.split("/")[-1]], document, path)
    if "const" in schema:
        require(type(value) is type(schema["const"]) and value == schema["const"], f"{path}: 常量不匹配")
    kind = schema.get("type")
    types = {"object": dict, "array": list, "string": str, "integer": int, "boolean": bool, "null": type(None)}
    if isinstance(kind, list):
        require(all(item in types for item in kind), f"{path}: 未知 schema 类型")
        matching = [item for item in kind if type(value) is types[item]]
        require(len(matching) == 1, f"{path}: 类型不匹配")
        kind = matching[0]
    if kind:
        require(kind in types and type(value) is types[kind], f"{path}: 应为 {kind}")
    if "enum" in schema:
        require(value in schema["enum"], f"{path}: 值不在允许集合中")
    if kind == "object":
        props = schema.get("properties", {})
        require(set(schema.get("required", [])) <= set(value), f"{path}: 缺少必需字段")
        if schema.get("additionalProperties") is False:
            require(set(value) <= set(props), f"{path}: 含未知字段 {sorted(set(value) - set(props))}")
        for key in value.keys() & props.keys():
            validate_schema(value[key], props[key], document, f"{path}.{key}")
    elif kind == "array":
        require(schema.get("minItems", 0) <= len(value) <= schema.get("maxItems", 1024), f"{path}: 数组大小不合法")
        if schema.get("uniqueItems"):
            require(len({canonical_bytes(item) for item in value}) == len(value), f"{path}: 数组值重复")
        for index, item in enumerate(value):
            validate_schema(item, schema["items"], document, f"{path}[{index}]")
    elif kind == "integer":
        require(schema.get("minimum", -9007199254740991) <= value <= schema.get("maximum", 9007199254740991), f"{path}: 数字越界")
    elif kind == "string":
        require(schema.get("minLength", 0) <= len(value) <= schema.get("maxLength", 1024), f"{path}: 字符串长度不合法")
        if "pattern" in schema:
            require(re.fullmatch(schema["pattern"], value) is not None, f"{path}: 字符串格式不合法")


def schema(name):
    return read_json(SCHEMAS / name)


def safe_relative(value):
    require(isinstance(value, str) and value and "\\" not in value and ":" not in value and not any(ord(c) < 32 for c in value), "制品路径不合法")
    parts = value.split("/")
    require(not PurePosixPath(value).is_absolute() and all(p not in ("", ".", "..") for p in parts), "制品必须使用规范化的受控相对路径")
    return value


def indexed(items, key, label):
    result = {item[key]: item for item in items}
    require(len(result) == len(items), f"{label} 身份重复")
    return result


def load_product(path=PRODUCT):
    with Path(path).open("rb") as stream:
        raw = stream.read(MAX_DOCUMENT + 1)
    require(len(raw) <= MAX_DOCUMENT, "产品 TOML 文档过大")
    try:
        product = tomllib.loads(raw.decode("utf-8"))
    except (UnicodeDecodeError, tomllib.TOMLDecodeError) as error:
        raise ReleaseError("产品 TOML 无法解析") from error
    validate_schema(product, schema("product.schema.json"))
    components = indexed(product["components"], "role", "产品组件")
    require(set(components) == {"control", "ime", "settings", "updater", "bootstrap"}, "产品必须声明 control/ime/settings/updater/bootstrap")
    require(len({c["bundle_id"] for c in components.values()}) == len(components), "产品 Bundle ID 重复")
    require("input_source_id" in components["ime"], "IME 缺少输入源 ID")
    for component in components.values():
        safe_relative(component["app_name"])
        require("/" not in component["app_name"] and component["app_name"].endswith(".app"), "App 名称必须为单一 .app 目录名")
    for channel in product["channels"].values():
        safe_relative(channel["feed_path"])
    require(len({c["feed_path"] for c in product["channels"].values()}) == 2, "渠道入口必须相互独立")
    return product


def validate_stores(stores, product):
    result = indexed(stores, "id", "数据库")
    require(set(result) == set(product["compatibility"]["required_stores"]), "逐库兼容合同未覆盖全部数据域")
    for entry in stores:
        for key in ("readable_schema_range", "writable_schema_range"):
            require(entry[key]["min"] <= entry[key]["max"], f"{entry['id']}: schema 范围倒置")
        read, write = entry["readable_schema_range"], entry["writable_schema_range"]
        require(read["min"] <= write["min"] <= write["max"] <= read["max"], "可写 schema 必须包含于可读范围")
        formats = entry["event_formats"]
        require(formats["writable_version"] in formats["readable_versions"], "写入的事件格式必须可读取")
        for kind in ("privacy", "revision", "outbox"):
            require(set(product["compatibility"][f"required_{kind}_capabilities"]) <= set(entry[f"{kind}_capabilities"]), f"{entry['id']}: 缺少 {kind} 合同")
    return result


def validate_manifest(value, product, *, expect_current_build=False):
    """仅验证结构与产品兼容声明；不表示签名或声明已通过实测。"""
    validate_schema(value, schema("release-manifest.schema.json"))
    require(value["product_id"] == product["product_id"], "manifest 产品身份不匹配")
    if expect_current_build:
        require(value["version"] == product["version"] and value["build"] == product["build"], "manifest 版本与本次产品构建不匹配")
    target = value["target"]
    require(all(os_version(v) >= os_version(target["min_os"]) for v in target["tested_os"]), "实测系统低于制品声明的最低系统")
    if expect_current_build:
        require(target["architecture"] in product["target"]["architectures"] and target["min_os"] == product["target"]["min_os"], "本次构建目标与产品支持合同不匹配")
    components = indexed(value["components"], "role", "组件")
    require({"control", "ime", "updater", "bootstrap"} <= set(components), "正式套件缺少控制中心、IME 或独立恢复组件")
    for expected in product["components"]:
        require(expected["role"] in components, "缺少产品组件")
        require(components[expected["role"]]["bundle_id"] == expected["bundle_id"], "组件 Bundle ID 与产品不匹配")
    teams = set()
    for component in components.values():
        require(component["bundle_id"] == component["signing_requirement"]["bundle_id"], "签名身份与组件不匹配")
        teams.add(component["signing_requirement"]["team_id"])
    require(len(teams) == 1, "同套产品必须来自同一 Developer ID 团队")
    current = validate_stores(value["stores"], product)
    rollbacks = indexed(value["rollback_targets"], "release_id", "回滚目标")
    require(value["release_id"] not in rollbacks, "回滚目标不能是自身")
    for rollback in rollbacks.values():
        previous = validate_stores(rollback["stores"], product)
        for sid, state in current.items():
            for kind in ("readable_schema_range", "writable_schema_range"):
                require(previous[sid][kind]["min"] <= state["writable_schema_range"]["min"] and previous[sid][kind]["max"] >= state["writable_schema_range"]["max"], f"回滚目标不能安全读写 {sid} 当前数据")
            require(state["event_formats"]["writable_version"] in previous[sid]["event_formats"]["readable_versions"], f"回滚目标无法消费 {sid} 当前事件")
            require(previous[sid]["event_formats"]["writable_version"] in state["event_formats"]["readable_versions"], f"当前程序无法读取 {sid} 回滚期间写入")
            for kind in ("privacy_capabilities", "revision_capabilities", "outbox_capabilities"):
                require(set(state[kind]) <= set(previous[sid][kind]), f"回滚目标缺少 {sid} 的 {kind}")
    indexed(value["resources"], "id", "资源")
    indexed(value["distribution_artifacts"], "role", "分发制品")
    require("installer-dmg" in {a["role"] for a in value["distribution_artifacts"]}, "缺少离线安装 DMG")
    all_artifacts = [*value["components"], *value["distribution_artifacts"], value["pair_manifest"]]
    paths = [safe_relative(a["artifact"]) for a in all_artifacts]
    require(len(set(paths)) == len(paths), "制品路径重复")
    return value


def validate_document(value, kind, product):
    if kind == "manifest":
        return validate_manifest(value, product)
    name = {"attestation": "release-attestation.schema.json", "feed": "channel-feed-v2.schema.json" if value.get("schema_version") == 2 else "channel-feed.schema.json"}[kind]
    validate_schema(value, schema(name))
    require(value["product_id"] == product["product_id"], "文档产品身份不匹配")
    if kind == "attestation":
        indexed(value["rollback_reports"], "release_id", "回滚报告")
    else:
        safe_relative(value["release_path"])
        require(parse_time(value["expires_at"]) > parse_time(value["issued_at"]), "feed 有效期不合法")
        if value["schema_version"] == 2:
            require(parse_time(value["expires_at"]) - parse_time(value["issued_at"]) <= dt.timedelta(days=7), "feed 有效期超过七天")
            require(value["release_path"] == f"releases/{value['release_id']}", "feed 必须指向准确的发布目录")
            if value["rollback"] is not None:
                rollback = value["rollback"]
                require(rollback["to_release_id"] == value["release_id"] and rollback["to_manifest_digest"] == value["manifest_digest"] and rollback["from_release_id"] != value["release_id"], "回滚身份不匹配")
    parse_time(value["issued_at"])
    return value


def parse_time(value):
    try:
        return dt.datetime.strptime(value, "%Y-%m-%dT%H:%M:%SZ").replace(tzinfo=dt.timezone.utc)
    except ValueError as error:
        raise ReleaseError("UTC 时间不合法") from error


def unwrap_document(value, kind, product):
    """返回已检查结构的 payload。返回值不表示 envelope 签名可信。"""
    require(type(value) is dict, "发布文档必须为 JSON 对象")
    if "payload" in value:
        validate_schema(value, schema("signed-envelope.schema.json"))
        require(value["payload_kind"] == kind, "签名信封类型不匹配")
        value = value["payload"]
    return validate_document(value, kind, product)


def _decode_b64(value, label):
    require(isinstance(value, str), f"{label} 必须是 Base64 字符串")
    try:
        decoded = base64.b64decode(value, validate=True)
    except (binascii.Error, ValueError) as error:
        raise ReleaseError(f"{label} 不是合法 Base64") from error
    require(decoded, f"{label} 不能为空")
    return decoded


def _trusted_keyset(path):
    value = read_json(path)
    require(type(value) is dict and set(value) == {"threshold", "keys"}, "受信密钥集字段不完整")
    require(type(value["threshold"]) is int and 1 <= value["threshold"] <= 32, "受信密钥阈值不合法")
    require(type(value["keys"]) is list and 1 <= len(value["keys"]) <= 32, "受信密钥集为空或过大")
    keys = {}
    for entry in value["keys"]:
        require(type(entry) is dict and set(entry) == {"key_id", "public_key_x963_base64"}, "受信密钥字段不完整")
        key_id = entry["key_id"]
        require(isinstance(key_id, str) and re.fullmatch(r"sha256-[0-9a-f]{64}", key_id) is not None, "受信密钥 ID 不合法")
        raw = _decode_b64(entry["public_key_x963_base64"], "受信公钥")
        require(len(raw) == 65 and raw[0] == 4, "受信公钥必须是 P-256 X9.63 未压缩格式")
        require(hashlib.sha256(raw).hexdigest() == key_id[7:], "受信密钥 ID 与公钥不匹配")
        require(key_id not in keys, "受信密钥 ID 重复")
        keys[key_id] = raw
    require(value["threshold"] <= len(keys), "受信密钥阈值超过密钥数量")
    return value["threshold"], keys


def verify_envelope_signature(value, kind, product, trusted_keyset_path):
    """使用显式提供的受信根密钥验证 envelope；信封自身不能建立信任。"""
    require(type(value) is dict and "payload" in value, "验签需要 signed envelope")
    validate_schema(value, schema("signed-envelope.schema.json"))
    require(value["payload_kind"] == kind, "签名信封类型不匹配")
    payload = validate_document(value["payload"], kind, product)
    threshold, keys = _trusted_keyset(trusted_keyset_path)
    signing = b"Inputia.Release.v1\0" + kind.encode("utf-8") + b"\0" + canonical_bytes(payload)
    spki_prefix = bytes.fromhex("3059301306072a8648ce3d020106082a8648ce3d030107034200")
    valid = set()
    with tempfile.TemporaryDirectory(prefix="inputia-verify-", dir=tempfile.gettempdir()) as directory:
        root = Path(directory)
        payload_path, signature_path, public_path = root / "payload", root / "signature.der", root / "public.der"
        payload_path.write_bytes(signing)
        for signature in value["signatures"]:
            key_id = signature["key_id"]
            if key_id not in keys or key_id in valid:
                continue
            raw_signature = _decode_b64(signature["signature_der_base64"], "签名")
            signature_path.write_bytes(raw_signature)
            public_path.write_bytes(spki_prefix + keys[key_id])
            result = subprocess.run(["openssl", "dgst", "-sha256", "-verify", str(public_path), "-keyform", "DER", "-signature", str(signature_path), str(payload_path)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, check=False, timeout=10)
            if result.returncode == 0 and result.stdout.strip() == "Verified OK":
                valid.add(key_id)
    require(len(valid) >= threshold, f"签名阈值未满足：{len(valid)}/{threshold}")
    return {"signature_verification": "PASS", "valid_signatures": len(valid), "threshold": threshold}


def sign_envelope(document_path, kind, private_key_path, output_path, product):
    """用调用方明确提供的私钥生成单签名 envelope；不生成发布授权。"""
    payload = read_json(document_path)
    require(type(payload) is dict and "payload" not in payload, "签名入口只接受未签名 payload")
    payload = validate_document(payload, kind, product)
    private_key = Path(private_key_path).resolve(strict=True)
    require(private_key.is_file() and not Path(private_key_path).is_symlink(), "私钥必须是普通文件且不能是符号链接")
    signing = b"Inputia.Release.v1\0" + kind.encode("utf-8") + b"\0" + canonical_bytes(payload)
    with tempfile.TemporaryDirectory(prefix="inputia-sign-", dir=tempfile.gettempdir()) as directory:
        root = Path(directory)
        signing_path, signature_path, public_path = root / "payload", root / "signature.der", root / "public.der"
        signing_path.write_bytes(signing)
        signed = subprocess.run(["openssl", "dgst", "-sha256", "-sign", str(private_key), "-out", str(signature_path), str(signing_path)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, check=False, timeout=10)
        require(signed.returncode == 0, "openssl 无法使用指定私钥签名")
        derived = subprocess.run(["openssl", "ec", "-in", str(private_key), "-pubout", "-outform", "DER", "-out", str(public_path)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, check=False, timeout=10)
        require(derived.returncode == 0, "openssl 无法从私钥导出公钥")
        public = public_path.read_bytes()
        require(len(public) >= 65 and public[-65] == 4, "私钥不是受支持的 P-256 公钥格式")
        raw_public = public[-65:]
        key_id = "sha256-" + hashlib.sha256(raw_public).hexdigest()
        envelope = {"schema_version": 1, "payload_kind": kind, "payload": payload, "signatures": [{"key_id": key_id, "algorithm": "ecdsa-p256-sha256", "signature_der_base64": base64.b64encode(signature_path.read_bytes()).decode("ascii")}]}
    validate_schema(envelope, schema("signed-envelope.schema.json"))
    write_file(output_path, (json.dumps(envelope, ensure_ascii=False, indent=2) + "\n").encode(), exclusive=True)
    return {"signed": True, "key_id": key_id, "document_sha256": file_digest(output_path), "public_release_eligible": False}


def file_digest(path):
    digest = hashlib.sha256()
    with Path(path).open("rb") as stream:
        while block := stream.read(1024 * 1024):
            digest.update(block)
    return digest.hexdigest()


def verify_artifacts(manifest, directory):
    root = Path(directory).resolve(strict=True)
    for artifact in [*manifest["components"], *manifest["distribution_artifacts"], manifest["pair_manifest"]]:
        relative = safe_relative(artifact["artifact"])
        path = root / relative
        require(path.resolve(strict=True).is_relative_to(root), "制品越出指定目录")
        cursor = root
        for part in Path(relative).parts:
            cursor = cursor / part
            require(not cursor.is_symlink(), "制品路径禁止符号链接")
        require(stat.S_ISREG(path.stat().st_mode), "摘要必须覆盖冻结的普通归档文件，不能是 .app 目录")
        if "size" in artifact:
            require(path.stat().st_size == artifact["size"], "制品大小不匹配")
        require(file_digest(path) == artifact["sha256"], "制品摘要不匹配")


def bind_manifest(template_path, context_path, artifact_dir, output_path, product=None):
    """将冻结制品摘要绑定到 manifest 模板；不执行签名或发布授权。"""
    product = product or load_product()
    template = read_json(template_path)
    validate_manifest(template, product)
    context = read_json(context_path)
    require(context.get("schema_version") == 1 and context.get("phase") == "prepared", "构建上下文不是已预检的冻结上下文")
    require(context.get("product_id") == product["product_id"], "构建上下文产品身份不匹配")
    require(context.get("product_digest") == hashlib.sha256(canonical_bytes(product)).hexdigest(), "构建上下文产品摘要不匹配")
    for key in ("release_id", "version", "build", "source_commit"):
        require(template.get(key) == context.get(key), f"manifest 与构建上下文的 {key} 不一致")
    require(template["target"] == context["target"], "manifest 与构建目标不一致")
    bound = copy.deepcopy(template)
    entries = [*bound["components"], *bound["distribution_artifacts"], bound["pair_manifest"]]
    root = Path(artifact_dir).resolve(strict=True)
    for entry in entries:
        relative = safe_relative(entry["artifact"])
        path = root / relative
        require(path.resolve(strict=True).is_relative_to(root), "制品越出冻结目录")
        cursor = root
        for part in Path(relative).parts:
            cursor = cursor / part
            require(not cursor.is_symlink(), "制品路径禁止符号链接")
        require(stat.S_ISREG(path.stat().st_mode), "制品必须是冻结的普通文件")
        entry["sha256"] = file_digest(path)
        if "size" in entry:
            entry["size"] = path.stat().st_size
    validate_manifest(bound, product, expect_current_build=True)
    verify_artifacts(bound, root)
    write_file(output_path, (json.dumps(bound, ensure_ascii=False, indent=2) + "\n").encode(), exclusive=True)
    return {"manifest_bound": True, "document_sha256": file_digest(output_path), "public_release_eligible": False}


def generated_files(product):
    control = next(c for c in product["components"] if c["role"] == "control")
    overlay = {"$schema": "https://schema.tauri.app/config/2", "productName": product["name"], "identifier": control["bundle_id"], "version": product["version"], "bundle": {"createUpdaterArtifacts": False, "targets": ["app"], "resources": ["resources/**/*"], "macOS": {"minimumSystemVersion": product["target"]["min_os"], "infoPlist": "InputiaReleaseInfo.plist", "signingIdentity": "-"}}}
    info = {"CFBundleDisplayName": product["name"], "CFBundleVersion": str(product["build"]), "NSMicrophoneUsageDescription": "Inputia 使用麦克风进行本地语音输入。"}
    metadata = {"schema_version": 1, "product_id": product["product_id"], "version": product["version"], "build": product["build"], "min_os": product["target"]["min_os"], "components": product["components"]}
    return {"src-tauri/tauri.inputia-release.conf.json": (json.dumps(overlay, ensure_ascii=False, indent=2) + "\n").encode(), "src-tauri/InputiaReleaseInfo.plist": plistlib.dumps(info, sort_keys=False), "release/generated/build-metadata.json": (json.dumps(metadata, ensure_ascii=False, indent=2) + "\n").encode()}


def config_drift(product, root=ROOT):
    drift = []
    for relative, expected in generated_files(product).items():
        path = root / relative
        try:
            if path.suffix == ".plist":
                matches = plistlib.loads(path.read_bytes()) == plistlib.loads(expected)
            else:
                matches = read_json(path) == json.loads(expected)
        except (OSError, ValueError, plistlib.InvalidFileException):
            matches = False
        if not matches:
            drift.append(relative)
    return drift


def write_file(path, data, *, exclusive=False):
    path = Path(path)
    require(not path.is_symlink(), "拒绝写入符号链接")
    require(path.parent.resolve() == path.parent.absolute(), "输出父目录禁止符号链接")
    path.parent.mkdir(parents=True, exist_ok=True)
    if exclusive:
        with path.open("xb") as stream:
            stream.write(data)
            stream.flush()
            os.fsync(stream.fileno())
    else:
        descriptor, temporary = tempfile.mkstemp(prefix=".inputia-", dir=path.parent)
        try:
            with os.fdopen(descriptor, "wb") as stream:
                stream.write(data)
                stream.flush()
                os.fsync(stream.fileno())
            os.replace(temporary, path)
        finally:
            if os.path.exists(temporary):
                os.unlink(temporary)


def git_state(root=ROOT):
    def git(*args):
        return subprocess.check_output(["git", "-C", str(root), *args], text=True).strip()
    commit = git("rev-parse", "--verify", "HEAD^{commit}")
    require(re.fullmatch(r"[0-9a-f]{40}|[0-9a-f]{64}", commit) is not None, "无法读取 Git 提交身份")
    return {"source_commit": commit, "working_tree_clean": not bool(git("status", "--porcelain", "--untracked-files=all"))}


def host_target():
    return {"platform": "macos" if sys.platform == "darwin" else sys.platform, "architecture": platform.machine()}


def _evidence_file(path):
    candidate = Path(path)
    require(candidate.is_absolute() and not candidate.is_symlink() and candidate.is_file(), "发布证据文件必须是绝对路径普通文件")
    require(candidate.stat().st_size <= MAX_DOCUMENT, "发布证据文件过大")
    return candidate


def _acceptance_pre_public_passes(path, source_commit):
    report = read_json(_evidence_file(path))
    require(isinstance(report, dict) and isinstance(report.get("subject"), dict), "验收报告结构不完整")
    require(report["subject"].get("source_commit") == source_commit, "验收报告提交身份不匹配")
    cases = {item.get("id"): item for item in report.get("cases", []) if isinstance(item, dict)}
    catalog = read_json(ROOT / "release/acceptance-cases.json")
    required = [case["id"] for case in catalog["cases"] if case["required"] and case["stage"] == "pre-public"]
    require(required and all(cases.get(case_id, {}).get("status") == "PASS" for case_id in required), "最终验收报告仍有未通过的 pre-public 案例")


def _verify_notarized_artifact(path):
    artifact = Path(path)
    require(artifact.is_absolute() and not artifact.is_symlink() and artifact.exists(), "公证制品路径无效")
    if sys.platform != "darwin":
        raise ReleaseError("公证制品只能在 macOS 主机核验")
    kind = "execute" if artifact.suffix == ".app" else "open"
    result = subprocess.run(["/usr/bin/codesign", "--verify", "--deep", "--strict", str(artifact)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, check=False, timeout=30)
    require(result.returncode == 0, "公证制品代码签名核验失败")
    result = subprocess.run(["/usr/sbin/spctl", "--assess", "--type", kind, "--context", "context:primary-signature", str(artifact)], stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True, check=False, timeout=30)
    require(result.returncode == 0, "公证制品 Gatekeeper 核验失败")


def validate_public_evidence(path, product, source_commit):
    """读取真实清单、受信密钥、最终制品和验收报告，不接受三项自报 PASS。"""
    value = read_json(path)
    require(type(value) is dict and value.get("schema_version") == 2, "公开发布证据版本不支持")
    require(value.get("product_id") == product["product_id"], "公开发布证据产品身份不匹配")
    require(value.get("source_commit") == source_commit, "公开发布证据提交身份不匹配")
    manifest = value.get("manifest")
    require(type(manifest) is dict, "公开发布证据缺少 manifest 引用")
    manifest_path = _evidence_file(manifest.get("path", ""))
    require(file_digest(manifest_path) == manifest.get("sha256"), "manifest 证据摘要不匹配")
    trusted_keys_path = _evidence_file(manifest.get("trusted_keys", ""))
    envelope = read_json(manifest_path)
    signature_result = verify_envelope_signature(envelope, "manifest", product, str(trusted_keys_path))
    require(signature_result.get("signature_verification") == "PASS", "签名 manifest 验证未通过")
    payload = unwrap_document(envelope, "manifest", product)
    require(payload["source_commit"] == source_commit, "签名 manifest 提交身份不匹配")
    verify_artifacts(payload, manifest.get("artifact_dir", ""))
    acceptance = value.get("acceptance")
    require(type(acceptance) is dict, "公开发布证据缺少 acceptance 引用")
    acceptance_path = _evidence_file(acceptance.get("path", ""))
    require(file_digest(acceptance_path) == acceptance.get("sha256"), "验收报告摘要不匹配")
    _acceptance_pre_public_passes(acceptance_path, source_commit)
    notarization = value.get("notarization")
    require(type(notarization) is dict, "公开发布证据缺少 notarization 引用")
    notarized = _evidence_file(notarization.get("artifact_path", ""))
    require(file_digest(notarized) == notarization.get("sha256"), "公证制品摘要不匹配")
    _verify_notarized_artifact(notarized)
    return value


def preflight(product, mode, root=ROOT, public_evidence=None):
    state = git_state(root)
    blockers = []
    host = host_target()
    if host["platform"] != product["target"]["platform"] or host["architecture"] not in product["target"]["architectures"]:
        blockers.append("unsupported_build_host")
    drift = config_drift(product, root)
    if drift:
        blockers.append("generated_config_drift")
    if mode == "public":
        if not state["working_tree_clean"]:
            blockers.append("dirty_release_checkout")
        if not product["pipeline"]["public_release_enabled"]:
            blockers.append("public_release_not_enabled")
        if product["pipeline"]["pair_trust_format"] < 2:
            blockers.append("profile_bound_pair_trust_v1")
        if public_evidence is None:
            blockers.extend(["developer_id_and_notarization_not_verified", "public_release_evidence_required", "final_artifact_acceptance_required"])
        else:
            try:
                validate_public_evidence(public_evidence, product, state["source_commit"])
            except (ReleaseError, OSError, ValueError):
                blockers.append("public_release_evidence_invalid")
    eligible = mode == "public" and not blockers
    return {"schema_version": 1, "mode": mode, "product_id": product["product_id"], "version": product["version"], "build": product["build"], **state, "host": host, "configuration_drift": drift, "blockers": blockers, "can_build": not blockers, "public_release_eligible": eligible, "certificate_accessed": False, "installed": False}


def prepare(product, output, mode, root=ROOT):
    report = preflight(product, mode, root)
    require(report["can_build"], "构建预检失败：" + ", ".join(report["blockers"]))
    release_id = f"inputia-{product['version']}-{product['build']}-{report['source_commit'][:12]}-{uuid.uuid4().hex}"
    context = {"schema_version": 1, "product_id": product["product_id"], "release_id": release_id, "version": product["version"], "build": product["build"], "source_commit": report["source_commit"], "working_tree_clean": report["working_tree_clean"], "mode": mode, "phase": "prepared", "created_at": dt.datetime.now(dt.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"), "target": product["target"], "python": sys.version.split()[0], "product_digest": hashlib.sha256(canonical_bytes(product)).hexdigest(), "public_release_eligible": False}
    directory = Path(output).absolute()
    require(not directory.exists(), "每次构建必须使用新的输出目录")
    require(directory.resolve() == directory, "构建输出必须为规范化路径，不能使用符号链接别名")
    for relative, data in generated_files(product).items():
        write_file(directory / Path(relative).name, data, exclusive=True)
    write_file(directory / "build-context.json", canonical_bytes(context) + b"\n", exclusive=True)
    return context


def apply_plist(path, role, context_path, product):
    context = read_json(context_path)
    require(context.get("phase") == "prepared" and context.get("mode") == "local", "当前仅允许已预检的本机 v1 构建")
    require(context.get("product_digest") == hashlib.sha256(canonical_bytes(product)).hexdigest(), "构建期间产品元数据已改变")
    require(context.get("source_commit") == git_state()["source_commit"], "构建期间 Git 提交已改变")
    require(re.fullmatch(r"inputia-[A-Za-z0-9._-]{1,180}", context.get("release_id", "")) is not None, "构建 release_id 不合法")
    component = next(c for c in product["components"] if c["role"] == role)
    path = Path(path).absolute()
    with path.open("rb") as stream:
        info = plistlib.load(stream)
    require(info.get("CFBundleIdentifier", component["bundle_id"]) == component["bundle_id"], "构建组件身份不匹配")
    info.update(CFBundleIdentifier=component["bundle_id"], CFBundleShortVersionString=product["version"], CFBundleVersion=str(product["build"]), LSMinimumSystemVersion=product["target"]["min_os"], InputiaReleaseID=context["release_id"], InputiaSourceCommit=context["source_commit"])
    info.pop("InputiaReleaseChannel", None)
    write_file(path, plistlib.dumps(info, sort_keys=False))


def os_version(value):
    parts = tuple(int(part) for part in value.split("."))
    return parts + (0,) * (3 - len(parts))


def verify_executable(path, product):
    architectures = subprocess.check_output(["/usr/bin/lipo", "-archs", str(path)], text=True).split()
    require(set(architectures) == set(product["target"]["architectures"]), "实际可执行架构与构建目标不匹配")
    commands = subprocess.check_output(["/usr/bin/otool", "-l", str(path)], text=True)
    versions = []
    command = None
    for line in commands.splitlines():
        fields = line.split()
        if len(fields) == 2 and fields[0] == "cmd":
            command = fields[1]
        if len(fields) == 2 and ((command == "LC_BUILD_VERSION" and fields[0] == "minos") or (command == "LC_VERSION_MIN_MACOSX" and fields[0] == "version")):
            versions.append(fields[1])
    require(len(versions) >= len(architectures) and all(os_version(v) <= os_version(product["target"]["min_os"]) for v in versions), "实际可执行最低系统高于产品承诺或缺少声明")
    return {"architectures": architectures, "minimum_system_versions": versions}


def verify_bundles(directory, context_path, product, scope="release"):
    require(scope in ("release", "local-legacy"), "未知包检查范围")
    context = read_json(context_path)
    require(context.get("product_digest") == hashlib.sha256(canonical_bytes(product)).hexdigest(), "构建元数据不匹配")
    results = []
    for component in product["components"]:
        if scope == "local-legacy" and component["role"] in ("updater", "bootstrap"):
            continue
        path = Path(directory) / component["app_name"] / "Contents/Info.plist"
        require(path.is_file() and not path.is_symlink(), f"{component['role']}: 缺少正式组件 Info.plist")
        bundle = path.parent.parent
        require(bundle.is_dir() and not bundle.is_symlink(), f"{component['role']}: 组件目录不是受管普通目录")
        with path.open("rb") as stream:
            info = plistlib.load(stream)
        expected = {"CFBundleIdentifier": component["bundle_id"], "CFBundleShortVersionString": product["version"], "CFBundleVersion": str(product["build"]), "InputiaReleaseID": context["release_id"], "InputiaSourceCommit": context["source_commit"], "LSMinimumSystemVersion": product["target"]["min_os"]}
        require(all(info.get(k) == v for k, v in expected.items()), f"{component['role']}: 实际产物元数据漂移")
        require("InputiaReleaseChannel" not in info, "制品不可固化渠道")
        executable = info.get("CFBundleExecutable", "")
        require(executable and "/" not in executable and "\\" not in executable and executable not in (".", ".."), "组件缺少安全的可执行文件名")
        binary = verify_executable(path.parent / "MacOS" / executable, product)
        results.append({"role": component["role"], "metadata_matches": True, "main_executable": binary})
    return {"scope": scope, "components": results, "code_signature_verification": "NOT_RUN", "notarization": "NOT_RUN", "public_release_eligible": False}


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--product", type=Path, default=PRODUCT)
    sub = parser.add_subparsers(dest="command", required=True)
    sub.add_parser("validate-product")
    generate = sub.add_parser("generate-config")
    generate.add_argument("--output-dir", type=Path, required=True)
    sub.add_parser("check-config")
    pre = sub.add_parser("preflight")
    pre.add_argument("--mode", choices=["local", "public"], default="local")
    pre.add_argument("--public-evidence", type=Path, help="受控签名/公证/最终验收流水线生成的绑定证据")
    prep = sub.add_parser("prepare")
    prep.add_argument("--mode", choices=["local", "public"], default="local")
    prep.add_argument("--output-dir", type=Path, required=True)
    val = sub.add_parser("validate")
    val.add_argument("--kind", choices=["manifest", "attestation", "feed"], required=True)
    val.add_argument("--document", type=Path, required=True)
    val.add_argument("--artifact-dir", type=Path)
    val.add_argument("--trusted-keys", type=Path, help="显式受信的 P-256 公钥集 JSON")
    sign = sub.add_parser("sign-envelope")
    sign.add_argument("--kind", choices=["manifest", "attestation", "feed"], required=True)
    sign.add_argument("--document", type=Path, required=True, help="未签名 payload JSON")
    sign.add_argument("--private-key", type=Path, required=True)
    sign.add_argument("--output", type=Path, required=True)
    apply = sub.add_parser("apply-plist")
    apply.add_argument("--plist", type=Path, required=True)
    apply.add_argument("--role", choices=["control", "ime", "settings", "updater", "bootstrap"], required=True)
    apply.add_argument("--context", type=Path, required=True)
    bundles = sub.add_parser("verify-bundles")
    bundles.add_argument("--directory", type=Path, required=True)
    bundles.add_argument("--context", type=Path, required=True)
    bundles.add_argument("--scope", choices=["release", "local-legacy"], default="release")
    bind = sub.add_parser("bind-manifest")
    bind.add_argument("--template", type=Path, required=True)
    bind.add_argument("--context", type=Path, required=True)
    bind.add_argument("--artifact-dir", type=Path, required=True)
    bind.add_argument("--output", type=Path, required=True)
    args = parser.parse_args(argv)
    try:
        product = load_product(args.product)
        code = 0
        if args.command == "validate-product":
            result = {"product_id": product["product_id"], "structure_valid": True, "public_release_eligible": False}
        elif args.command == "generate-config":
            for relative, data in generated_files(product).items():
                write_file(args.output_dir.absolute() / relative, data)
            result = {"generated": list(generated_files(product))}
        elif args.command == "check-config":
            result = {"configuration_drift": config_drift(product)}
            code = 1 if result["configuration_drift"] else 0
        elif args.command == "preflight":
            result = preflight(product, args.mode, public_evidence=args.public_evidence)
            code = 1 if result["blockers"] else 0
        elif args.command == "prepare":
            result = prepare(product, args.output_dir, args.mode)
        elif args.command == "apply-plist":
            apply_plist(args.plist, args.role, args.context, product)
            result = {"metadata_applied": True, "role": args.role}
        elif args.command == "verify-bundles":
            result = verify_bundles(args.directory, args.context, product, args.scope)
        elif args.command == "bind-manifest":
            result = bind_manifest(args.template, args.context, args.artifact_dir, args.output, product)
        elif args.command == "sign-envelope":
            result = sign_envelope(args.document, args.kind, args.private_key, args.output, product)
        else:
            value = unwrap_document(read_json(args.document), args.kind, product)
            if args.artifact_dir:
                require(args.kind == "manifest", "仅 manifest 支持制品摘要检查")
                verify_artifacts(value, args.artifact_dir)
            signature_result = {"signature_verification": "NOT_RUN"}
            if args.trusted_keys:
                signature_result = verify_envelope_signature(read_json(args.document), args.kind, product, args.trusted_keys)
            result = {"structure_valid": True, "artifact_verification": "PASS" if args.artifact_dir else "NOT_RUN", **signature_result, "public_release_eligible": False, "document_sha256": file_digest(args.document)}
        print(json.dumps(result, ensure_ascii=False, indent=2))
        return code
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        print(json.dumps({"error": str(error), "public_release_eligible": False}, ensure_ascii=False), file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
