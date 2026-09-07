#!/usr/bin/env python3
"""离线候选构建常量；只接收公钥，绝不读取或分发签名私钥。"""
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


def create(public_key, run_id):
    if not re.fullmatch(r"[A-Za-z0-9_-]{1,64}", run_id):
        raise ValueError("invalid candidate run ID")
    if len(public_key) != 65 or public_key[0] != 4:
        raise ValueError("expected uncompressed P256 public key, never private representation")
    # 曲线有效性由真实Security验签再次验证；这里验证构建数据结构。
    return {
        "schema_version": 1,
        "run_id": run_id,
        "profile_id": "unified-candidate:" + run_id,
        "key_id": "candidate-" + hashlib.sha256(public_key).hexdigest(),
        "public_key_x963_hex": public_key.hex(),
    }


def load(path, run_id):
    data = json.loads(checked_file(path, 4096))
    if not isinstance(data, dict) or set(data) != {"schema_version", "run_id", "profile_id", "key_id", "public_key_x963_hex"}:
        raise ValueError("unexpected build trust fields")
    key = bytes.fromhex(data["public_key_x963_hex"])
    expected = create(key, run_id)
    if data != expected or type(data["schema_version"]) is not int:
        raise ValueError("build trust identity does not match candidate")
    return expected, key


def source(metadata, key, language):
    values = ", ".join(str(byte) for byte in key)
    if language == "rust":
        return (
            "// 构建期生成，公钥在签名前编入二进制。\n"
            f"pub static PUBLIC_KEY: [u8; 65] = [{values}];\n"
            f'pub const KEY_ID: &str = "{metadata["key_id"]}";\n'
            f'pub const RUN_ID: &str = "{metadata["run_id"]}";\n'
            f'pub const PROFILE_ID: &str = "{metadata["profile_id"]}";\n'
        )
    if language == "swift":
        return (
            "// 构建期生成，公钥在签名前编入二进制。\nimport Foundation\n"
            "enum InputiaEmbeddedPairTrust {\n"
            f"  static let publicKey = Data([{values}])\n"
            f'  static let keyID = "{metadata["key_id"]}"\n'
            f'  static let runID = "{metadata["run_id"]}"\n'
            f'  static let profileID = "{metadata["profile_id"]}"\n'
            "  static var trust: PairTrust {\n"
            "    PairTrust(publicKeyX963: publicKey, keyID: keyID, runID: runID,\n"
            "      profileID: profileID, protocolMajor: 1, localRole: .inputia)\n"
            "  }\n}\n"
        )
    raise ValueError("unsupported source language")


def write_new(path, data):
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "w") as stream:
        stream.write(data)
        stream.flush()
        os.fsync(stream.fileno())


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--run-id", required=True)
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
    if args.sign_pair:
        if args.emit or args.public_key or not all([args.handy, args.inputia, args.build_tool, args.private_key, args.manifest]):
            parser.error("sign-pair requires both signed apps, build tool, private key and new manifest path")
        metadata, _ = load(args.metadata, args.run_id)
        peers = [json.loads(subprocess.check_output([str(args.build_tool), "identity", role, str(path)]))
                 for role, path in [("handy", args.handy), ("inputia", args.inputia)]]
        payload = {"schemaVersion": 1, "mode": "candidate", "keyID": metadata["key_id"],
                   "runID": metadata["run_id"], "profileID": metadata["profile_id"], "protocolMajor": 1, "peers": peers}
        with tempfile.TemporaryDirectory(prefix="pair-payload-") as temporary:
            path = Path(temporary) / "payload.json"
            write_new(path, json.dumps(payload, ensure_ascii=False, sort_keys=True, separators=(",", ":")))
            subprocess.run([str(args.build_tool), "sign", str(args.private_key), str(path), str(args.manifest)], check=True)
        return
    if args.public_key is not None:
        if args.emit:
            parser.error("creation and emission are separate build steps")
        metadata = create(checked_file(args.public_key, 65), args.run_id)
        write_new(args.metadata, json.dumps(metadata, sort_keys=True, indent=2) + "\n")
    else:
        if not args.emit:
            parser.error("emission format required")
        metadata, key = load(args.metadata, args.run_id)
        print(source(metadata, key, args.emit), end="")


if __name__ == "__main__":
    main()
