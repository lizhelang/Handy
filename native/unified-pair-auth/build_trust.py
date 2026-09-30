#!/usr/bin/env python3
"""离线配对构建常量；兼容 Python 3.9+，不依赖 TOML 或 Keychain。"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import subprocess
import tempfile


def checked_file(path, limit):
    path = Path(path)
    if not path.is_absolute() or path.resolve() != path:
        raise ValueError("build trust input must be canonical and absolute")
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW)
    try:
        metadata = os.fstat(descriptor)
        if not stat.S_ISREG(metadata.st_mode) or metadata.st_nlink != 1 or metadata.st_uid != os.getuid() or metadata.st_mode & 0o022:
            raise ValueError("build trust input has unsafe type, owner or permissions")
        data = os.read(descriptor, limit + 1)
        if len(data) > limit:
            raise ValueError("build trust input too large")
        return data
    finally:
        os.close(descriptor)


def strict_json(raw):
    def pairs(items):
        result = {}
        for key, value in items:
            if key in result:
                raise ValueError("duplicate JSON key: " + key)
            result[key] = value
        return result

    def nonfinite(value):
        raise ValueError("nonfinite JSON number: " + value)
    return json.loads(raw, object_pairs_hook=pairs, parse_constant=nonfinite)


def validate_key(public_key):
    if len(public_key) != 65 or public_key[0] != 4:
        raise ValueError("expected uncompressed P256 public key, never private representation")


def create(public_key, run_id):
    if not isinstance(run_id, str) or not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", run_id):
        raise ValueError("invalid candidate run ID")
    validate_key(public_key)
    return {"schema_version": 1, "run_id": run_id,
            "profile_id": "unified-candidate:" + run_id,
            "key_id": "candidate-" + hashlib.sha256(public_key).hexdigest(),
            "public_key_x963_hex": public_key.hex()}


def release_context(path):
    raw = checked_file(path, 16_384)
    context = strict_json(raw)
    fields = {"schema_version", "product_id", "release_id", "version", "build", "source_commit",
              "working_tree_clean", "mode", "phase", "created_at", "target", "python",
              "product_digest", "public_release_eligible"}
    if not isinstance(context, dict) or set(context) != fields:
        raise ValueError("unexpected build context fields")
    if (type(context["schema_version"]) is not int or context["schema_version"] != 1
            or context["product_id"] != "com.inputia" or context["phase"] != "prepared"
            or context["mode"] not in ["local", "public"]
            or type(context["working_tree_clean"]) is not bool
            or context["public_release_eligible"] is not False
            or not isinstance(context["source_commit"], str)
            or not re.fullmatch(r"[0-9a-f]{40}", context["source_commit"])
            or not isinstance(context["product_digest"], str)
            or not re.fullmatch(r"[0-9a-f]{64}", context["product_digest"])
            or not isinstance(context["version"], str)
            or not re.fullmatch(r"[0-9]+\.[0-9]+\.[0-9]+", context["version"])
            or type(context["build"]) is not int or context["build"] < 1):
        raise ValueError("invalid prepared release context")
    prefix = "inputia-{}-{}-{}-".format(context["version"], context["build"], context["source_commit"][:12])
    if not isinstance(context["release_id"], str) or not re.fullmatch(re.escape(prefix) + r"[0-9a-f]{32}", context["release_id"]):
        raise ValueError("release ID is not bound to the prepared build")
    return context, hashlib.sha256(raw).hexdigest()


def create_release(public_key, context_path):
    validate_key(public_key)
    context, digest = release_context(context_path)
    return {"schema_version": 2, "product_id": context["product_id"], "release_id": context["release_id"],
            "protocol_major": 1, "source_commit": context["source_commit"], "context_sha256": digest,
            "key_id": "release-" + hashlib.sha256(public_key).hexdigest(),
            "public_key_x963_hex": public_key.hex()}


def load(path, run_id=None, context_path=None):
    if (run_id is None) == (context_path is None):
        raise ValueError("select exactly one legacy run or release context")
    data = strict_json(checked_file(path, 4096))
    if not isinstance(data, dict) or not isinstance(data.get("public_key_x963_hex"), str):
        raise ValueError("unexpected build trust fields")
    key = bytes.fromhex(data["public_key_x963_hex"])
    expected = create_release(key, context_path) if context_path is not None else create(key, run_id)
    if data != expected or type(data.get("schema_version")) is not int or (
            context_path is not None and type(data.get("protocol_major")) is not int):
        raise ValueError("build trust identity does not match build context")
    return expected, key


def source(metadata, key, language):
    values = ", ".join(str(byte) for byte in key)
    release = metadata["schema_version"] == 2
    if language == "rust":
        binding = 'Some(("{}", "{}", 1))'.format(metadata["product_id"], metadata["release_id"]) if release else "None"
        legacy = "None" if release else 'Some(("{}", "{}"))'.format(metadata["run_id"], metadata["profile_id"])
        return ("// 构建期生成，公钥在签名前编入二进制。\n"
                f"pub static PUBLIC_KEY: [u8; 65] = [{values}];\n"
                f'pub const KEY_ID: &str = "{metadata["key_id"]}";\n'
                f"pub const LEGACY_PROFILE: Option<(&str, &str)> = {legacy};\n"
                f"pub const RELEASE_BINDING: Option<(&str, &str, u16)> = {binding};\n")
    if language == "swift":
        common = ("// 构建期生成，公钥在签名前编入二进制。\nimport Foundation\n"
                  "enum InputiaEmbeddedPairTrust {\n"
                  f"  static let publicKey = Data([{values}])\n"
                  f'  static let keyID = "{metadata["key_id"]}"\n')
        if release:
            return common + (f'  static let productID = "{metadata["product_id"]}"\n'
                             f'  static let releaseID = "{metadata["release_id"]}"\n'
                             "  static var trust: PairReleaseTrust {\n"
                             "    PairReleaseTrust(publicKeyX963: publicKey, keyID: keyID, productID: productID,\n"
                             "      releaseID: releaseID, protocolMajor: 1, localRole: .inputia)\n"
                             "  }\n}\n")
        return common + (f'  static let runID = "{metadata["run_id"]}"\n'
                         f'  static let profileID = "{metadata["profile_id"]}"\n'
                         "  static var trust: PairTrust {\n"
                         "    PairTrust(publicKeyX963: publicKey, keyID: keyID, runID: runID,\n"
                         "      profileID: profileID, protocolMajor: 1, localRole: .inputia)\n"
                         "  }\n}\n")
    raise ValueError("unsupported source language")


def write_new(path, data):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "w") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def main():
    parser = argparse.ArgumentParser()
    binding = parser.add_mutually_exclusive_group(required=True)
    binding.add_argument("--run-id")
    binding.add_argument("--release-context", type=Path)
    parser.add_argument("--public-key", type=Path)
    parser.add_argument("--metadata", type=Path, required=True)
    parser.add_argument("--emit", choices=["rust", "swift"])
    parser.add_argument("--sign-pair", action="store_true")
    parser.add_argument("--handy", type=Path)
    parser.add_argument("--inputia", type=Path)
    parser.add_argument("--build-tool", type=Path)
    parser.add_argument("--private-key", type=Path)
    parser.add_argument("--manifest", type=Path)
    args = parser.parse_args()
    if args.release_context:
        context, _ = release_context(args.release_context)
        commit = subprocess.check_output(["git", "-C", str(Path(__file__).resolve().parents[2]), "rev-parse", "HEAD"], text=True).strip()
        if context["source_commit"] != commit:
            parser.error("source commit changed since release preparation")
    if args.sign_pair:
        if args.emit or args.public_key or not all([args.handy, args.inputia, args.build_tool, args.private_key, args.manifest]):
            parser.error("sign-pair requires both signed apps, build tool, private key and new manifest path")
        metadata, key = load(args.metadata, args.run_id, args.release_context)
        # 先核对临时签名器公钥，避免错误私钥生成无法由嵌入信任验证的制品。
        signer_key = subprocess.check_output([str(args.build_tool), "public-key", str(args.private_key)], text=True).strip()
        if signer_key != key.hex():
            parser.error("signer public key does not match embedded build trust")
        peers = [strict_json(subprocess.check_output([str(args.build_tool), "identity", role, str(path)]))
                 for role, path in [("handy", args.handy), ("inputia", args.inputia)]]
        if args.release_context:
            payload = {"schemaVersion": 2, "productID": metadata["product_id"], "releaseID": metadata["release_id"],
                       "keyID": metadata["key_id"], "protocolMajor": 1, "peers": peers}
            command = "sign-release"
        else:
            payload = {"schemaVersion": 1, "mode": "candidate", "keyID": metadata["key_id"],
                       "runID": metadata["run_id"], "profileID": metadata["profile_id"], "protocolMajor": 1, "peers": peers}
            command = "sign"
        with tempfile.TemporaryDirectory(prefix="pair-payload-") as temporary:
            path = Path(temporary) / "payload.json"
            write_new(path, json.dumps(payload, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
            subprocess.run([str(args.build_tool), command, str(args.private_key), str(path), str(args.manifest)], check=True)
        return
    if args.public_key is not None:
        if args.emit:
            parser.error("creation and emission are separate build steps")
        key = checked_file(args.public_key, 65)
        metadata = create_release(key, args.release_context) if args.release_context else create(key, args.run_id)
        write_new(args.metadata, json.dumps(metadata, sort_keys=True, indent=2) + "\n")
    else:
        if not args.emit:
            parser.error("emission format required")
        metadata, key = load(args.metadata, args.run_id, args.release_context)
        print(source(metadata, key, args.emit), end="")


if __name__ == "__main__":
    main()
