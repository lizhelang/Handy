#!/usr/bin/env python3
"""Inputia 发布验收账本：缺失、跳过和间接证据不能使发布门槛通过。"""

import argparse
import copy
import hashlib
import json
import math
import platform
import re
import subprocess
import sys
from datetime import datetime, timezone
from pathlib import Path, PurePosixPath
from uuid import uuid4

ROOT = Path(__file__).resolve().parents[1]
CATALOG = ROOT / "release/acceptance-cases.json"
STAGES = ("pre-public", "candidate", "stable")
STATUSES = ("PASS", "FAIL", "BLOCKED", "NOT_RUN", "NOT_APPLICABLE")
LEVELS = ("source", "unit", "integration", "ui_mock", "native_api", "physical_input",
          "clean_machine", "independent_review", "artifact", "distribution", "observation")
HEX = re.compile(r"[0-9a-f]{64}\Z")
COMMIT = re.compile(r"(?:[0-9a-f]{40}|[0-9a-f]{64})\Z")


class AcceptanceError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise AcceptanceError(message)


def fields(value, expected, label):
    require(type(value) is dict and set(value) == set(expected), f"{label}: 字段缺失或未知")


def nonempty(value):
    return isinstance(value, str) and bool(value.strip())


def _pairs(pairs):
    result = {}
    for key, value in pairs:
        require(key not in result, "JSON 含重复字段")
        result[key] = value
    return result


def read_json(path):
    def invalid_constant(_):
        raise AcceptanceError("JSON 含非有限数值")
    return json.loads(Path(path).read_text(encoding="utf-8"), object_pairs_hook=_pairs,
                      parse_constant=invalid_constant)


def digest(path):
    h = hashlib.sha256()
    with Path(path).open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            h.update(chunk)
    return h.hexdigest()


def now():
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def timestamp(value):
    require(isinstance(value, str) and value.endswith("Z"), "时间必须为 UTC ISO-8601")
    try:
        parsed = datetime.fromisoformat(value[:-1] + "+00:00")
    except ValueError as error:
        raise AcceptanceError("无效时间") from error
    require(parsed <= datetime.now(timezone.utc), "验收时间不能在未来")
    return parsed


def catalog():
    data = read_json(CATALOG)
    fields(data, ("schema_version", "plan_id", "cases"), "catalog")
    require(type(data["schema_version"]) is int and data["schema_version"] == 1,
            "不支持的案例目录版本")
    require(data["plan_id"] == "P20260930-191324", "案例目录不属于已批准计划")
    require(isinstance(data["cases"], list) and data["cases"], "案例目录为空")
    ids = set()
    for case in data["cases"]:
        fields(case, ("id", "gate", "stage", "evidence_levels", "description", "required", "metrics"), "case")
        require(nonempty(case["id"]) and case["id"] not in ids, "案例 ID 重复或无效")
        ids.add(case["id"])
        require(case["stage"] in STAGES and case["required"] is True, "无效阶段或必需标记")
        require(isinstance(case["evidence_levels"], list) and case["evidence_levels"]
                and set(case["evidence_levels"]) <= set(LEVELS), "无效证据等级")
        require(type(case["metrics"]) is dict, "无效指标合同")
        for bounds in case["metrics"].values():
            require(type(bounds) is dict and {"min", "type"} <= set(bounds)
                    and set(bounds) <= {"min", "max", "type"} and bounds["type"] in ("integer", "number"), "无效指标界限")
            require(all(type(n) in (int, float) and math.isfinite(n) for k, n in bounds.items() if k != "type"), "无效指标数值")
    require({case["gate"] for case in data["cases"]} == {"G" + str(i) for i in range(10)} | {"G10a", "G10b"},
            "案例目录未覆盖所有发布门槛")
    return data


def artifact_file(root, value):
    relative = relative_path(value)
    root = Path(root).resolve()
    candidate = root.joinpath(relative)
    require(candidate.resolve().is_relative_to(root), "最终制品路径越界")
    current = root
    for part in relative.parts:
        current = current / part
        require(not current.is_symlink(), "最终制品不能经过符号链接")
    require(candidate.is_file(), "最终制品缺失或不是普通文件")
    return candidate


def verify_distribution_artifacts(subject, root):
    """逐文件核对 manifest 的分发制品摘要和大小；不接受清单自报作为证据。"""
    for artifact in subject["artifacts"]:
        file = artifact_file(root, artifact["artifact"])
        size = file.stat().st_size
        require(size == artifact["size"], f"最终制品大小不符：{artifact['artifact']}")
        require(digest(file) == artifact["sha256"], f"最终制品摘要不符：{artifact['artifact']}")


def manifest_subject(path, artifact_root=None):
    # 这里只校验结构与绑定；签名、公证与放行由 G10a 及发布器验证。
    from inputia_release import load_product, unwrap_document
    document = read_json(path)
    payload = unwrap_document(document, "manifest", load_product())
    artifacts = [{key: item[key] for key in ("role", "artifact", "sha256", "size")}
                 for item in payload["distribution_artifacts"]]
    require(artifacts, "验收必须绑定最终分发制品")
    subject = {"product_id": payload["product_id"], "release_id": payload["release_id"],
            "source_commit": payload["source_commit"], "manifest_sha256": digest(path),
            "target": payload["target"],
            "artifacts": artifacts}
    if artifact_root is not None:
        verify_distribution_artifacts(subject, artifact_root)
    return subject


def validate_subject(subject):
    fields(subject, ("product_id", "release_id", "source_commit", "manifest_sha256", "target", "artifacts"), "subject")
    require(nonempty(subject["product_id"]) and nonempty(subject["release_id"]), "缺少产品/发布身份")
    require(isinstance(subject["source_commit"], str) and COMMIT.fullmatch(subject["source_commit"]), "无效源码提交")
    require(isinstance(subject["manifest_sha256"], str) and HEX.fullmatch(subject["manifest_sha256"]), "无效清单摘要")
    fields(subject["target"], ("platform", "architecture", "min_os", "tested_os"), "target")
    require(subject["target"]["platform"] == "macos" and subject["target"]["architecture"] == "arm64", "验收目录只覆盖首发 macOS arm64 范围")
    version(subject["target"]["min_os"])
    require(isinstance(subject["target"]["tested_os"], list), "无效受验系统矩阵")
    for item in subject["target"]["tested_os"]:
        version(item)
    from inputia_release import load_product
    declared = load_product()["target"]
    require(version(subject["target"]["min_os"]) == version(declared["min_os"]), "最低支持系统与产品声明不同")
    require(subject["target"]["tested_os"] and max(map(version, subject["target"]["tested_os"]))[0]
            >= max(map(version, declared["declared_os"]))[0], "当前支持系统证据矩阵被降低")
    require(isinstance(subject["artifacts"], list) and subject["artifacts"], "缺少最终制品")
    names = set()
    for artifact in subject["artifacts"]:
        fields(artifact, ("role", "artifact", "sha256", "size"), "artifact")
        relative_path(artifact["artifact"])
        require(artifact["artifact"] not in names, "制品路径重复")
        names.add(artifact["artifact"])
        require(nonempty(artifact["role"]) and isinstance(artifact["sha256"], str)
                and HEX.fullmatch(artifact["sha256"]), "无效制品摘要或角色")
        require(type(artifact["size"]) is int and artifact["size"] > 0, "无效制品大小")


def initial_report(subject):
    validate_subject(subject)
    definition = catalog()
    return {"schema_version": 1, "plan_id": definition["plan_id"],
            "catalog_sha256": digest(CATALOG), "subject": copy.deepcopy(subject), "created_at": now(),
            "cases": [{"id": case["id"], "status": "NOT_RUN", "evidence_level": None,
                       "reason": "尚未运行", "procedure": [], "started_at": None, "executed_at": None,
                       "machine": None, "evidence": [], "execution_record": None,
                       "metrics": {}} for case in definition["cases"]]}


def relative_path(value):
    require(nonempty(value) and "\\" not in value and "\x00" not in value, "证据路径无效")
    path = PurePosixPath(value)
    require(not path.is_absolute() and all(p not in ("", ".", "..") for p in value.split("/")), "路径必须为受控相对路径")
    require(":" not in value, "路径不能包含卷或协议前缀")
    return path


def version(value):
    require(isinstance(value, str) and re.fullmatch(r"\d+(?:\.\d+){0,2}", value), "无效系统版本")
    parts = tuple(map(int, value.split(".")))
    return parts + (0,) * (3 - len(parts))


def evidence_file(root, value):
    relative = relative_path(value)
    root = Path(root).resolve()
    candidate = root.joinpath(relative)
    require(candidate.resolve().is_relative_to(root), "证据路径越界")
    current = root
    for part in relative.parts:
        current = current / part
        require(not current.is_symlink(), "证据不能经过符号链接")
    require(candidate.is_file(), "证据文件缺失或不是普通文件")
    return candidate


def validate_report(report, root, expected_subject=None):
    fields(report, ("schema_version", "plan_id", "catalog_sha256", "subject", "created_at", "cases"), "report")
    require(type(report["schema_version"]) is int and report["schema_version"] == 1, "不支持的报告版本")
    definition = catalog()
    require(report["plan_id"] == definition["plan_id"] and report["catalog_sha256"] == digest(CATALOG), "报告案例目录与当前合同不同")
    validate_subject(report["subject"])
    if expected_subject is not None:
        require(report["subject"] == expected_subject, "报告不属于本次源码/清单/制品")
    timestamp(report["created_at"])
    definitions = {case["id"]: case for case in definition["cases"]}
    require(isinstance(report["cases"], list), "案例必须为数组")
    seen = set()
    for result in report["cases"]:
        fields(result, ("id", "status", "evidence_level", "reason", "procedure", "started_at", "executed_at", "machine", "evidence", "execution_record", "metrics"), "result")
        identity = result["id"]
        require(isinstance(identity, str) and identity in definitions and identity not in seen, "未知/重复案例")
        seen.add(identity)
        case = definitions[identity]
        require(result["status"] in STATUSES and nonempty(result["reason"]), "无效状态或缺少解释")
        require(result["evidence_level"] is None or result["evidence_level"] in LEVELS, "无效证据等级")
        require(isinstance(result["procedure"], list) and all(nonempty(p) for p in result["procedure"]), "无效步骤")
        require(isinstance(result["evidence"], list) and type(result["metrics"]) is dict, "无效证据或指标")
        for key, value in result["metrics"].items():
            require(key in case["metrics"] and type(value) in (int, float) and math.isfinite(value), "未知或无效指标")
            require(case["metrics"][key]["type"] != "integer" or type(value) is int, "计数必须为整数")
            require(value >= case["metrics"][key]["min"], "指标未达标或低于合法下界")
        if result["executed_at"] is not None:
            timestamp(result["executed_at"])
        if result["started_at"] is not None:
            timestamp(result["started_at"])
        if result["machine"] is not None:
            fields(result["machine"], ("os", "os_version", "architecture", "model"), "machine")
            require(all(nonempty(v) for v in result["machine"].values()), "机器信息缺失")
        paths = set()
        for evidence in result["evidence"]:
            fields(evidence, ("path", "sha256"), "evidence")
            require(isinstance(evidence["sha256"], str) and HEX.fullmatch(evidence["sha256"]), "无效证据摘要")
            file = evidence_file(root, evidence["path"])
            require(evidence["path"] not in paths, "证据重复")
            paths.add(evidence["path"])
            require(digest(file) == evidence["sha256"], "证据已修改或摘要错误")
        if result["status"] == "NOT_RUN":
            require(result["evidence_level"] is None and result["executed_at"] is None and result["started_at"] is None
                    and result["machine"] is None and not result["procedure"]
                    and not result["evidence"] and not result["metrics"] and result["execution_record"] is None, "未运行案例不能携带执行结果")
        if result["status"] == "NOT_APPLICABLE":
            raise AcceptanceError("当前首发目录所有案例均必需，不能用不适用跳过")
        if result["status"] == "PASS":
            require(result["evidence_level"] in case["evidence_levels"], f"{identity}: 证据等级不能证明该要求")
            require(result["procedure"] and result["started_at"] and result["executed_at"] and result["machine"] and result["evidence"], f"{identity}: 通过缺少执行证据")
            elapsed = (timestamp(result["executed_at"]) - timestamp(result["started_at"])).total_seconds()
            require(elapsed >= 0, "执行结束早于开始")
            for unit, seconds in (("duration_days", 86400), ("duration_hours", 3600), ("duration_seconds", 1)):
                if unit in result["metrics"]:
                    require(elapsed >= result["metrics"][unit] * seconds, f"{identity}: 实际观察时间不足")
            target = report["subject"]["target"]
            if result["evidence_level"] in ("native_api", "physical_input", "clean_machine"):
                require(result["machine"]["os"] == target["platform"]
                        and result["machine"]["architecture"] == target["architecture"], "原生证据机器不属于目标平台")
                os_version = version(result["machine"]["os_version"])
                require(os_version >= version(target["min_os"]), "原生证据系统低于声明范围")
                if identity == "G8.minimum-os":
                    require(os_version[0] == version(target["min_os"])[0], "缺少最低支持系统实测")
                if identity == "G8.current-os":
                    require(target["tested_os"] and os_version == max(map(version, target["tested_os"])), "缺少当前支持系统实测")
            require(set(result["metrics"]) == set(case["metrics"]), f"{identity}: 缺少验收指标")
            for key, bounds in case["metrics"].items():
                value = result["metrics"][key]
                require(("min" not in bounds or value >= bounds["min"])
                        and ("max" not in bounds or value <= bounds["max"]), f"{identity}: 指标 {key} 未达标")
            validate_execution_record(result, report["subject"], root)
    require(seen == set(definitions), "报告缺少必需案例；不能以部分通过放行")
    by_id = {case["id"]: case for case in report["cases"]}
    candidate = by_id["G10b.candidate-download"]
    observation = by_id["G10b.observation"]
    stable = by_id["G10b.stable-download"]
    if candidate["status"] == "PASS":
        for case in definition["cases"]:
            result = by_id[case["id"]]
            if case["stage"] == "pre-public" and result["status"] == "PASS":
                require(timestamp(candidate["started_at"]) >= timestamp(result["executed_at"]), "候选下载验证早于发布前验收")
    if observation["status"] == "PASS" and candidate["status"] == "PASS":
        require(timestamp(observation["started_at"]) >= timestamp(candidate["executed_at"]), "观察期早于候选制品可用验证")
    if stable["status"] == "PASS" and observation["status"] == "PASS":
        require(timestamp(stable["started_at"]) >= timestamp(observation["executed_at"]), "稳定晋级早于候选观察完成")
    return report


def execution_payload(result, subject, producer):
    """执行记录不含自身摘要，避免与聚合报告互相引用。"""
    keys = ("id", "status", "evidence_level", "procedure", "started_at", "executed_at", "machine", "metrics", "evidence")
    return {"schema_version": 1, "subject": copy.deepcopy(subject), "producer": producer,
            "result": {key: copy.deepcopy(result[key]) for key in keys}}


def validate_execution_record(result, subject, root):
    reference = result["execution_record"]
    fields(reference, ("path", "sha256"), "execution_record")
    path = evidence_file(root, reference["path"])
    require(digest(path) == reference["sha256"], "执行记录摘要错误")
    record = read_json(path)
    fields(record, ("schema_version", "subject", "producer", "result"), "execution")
    require(type(record["schema_version"]) is int and record["schema_version"] == 1, "执行记录版本无效")
    producer = record["producer"]
    fields(producer, ("kind", "identity", "reviewer"), "producer")
    if producer["kind"] == "builtin":
        require(producer["identity"] == "inputia-rust-contracts/v2" and result["id"] == "G1.rust"
                and producer["reviewer"] is None, "执行器无权证明该案例")
    else:
        require(producer["kind"] == "reviewed" and nonempty(producer["identity"])
                and nonempty(producer["reviewer"]) and producer["reviewer"] != producer["identity"], "手工/外部执行证据缺少独立复核记录")
    require(record == execution_payload(result, subject, producer), "执行证据未绑定同一案例、制品、指标或时段")


def summarize(report, stage):
    require(stage in STAGES, "未知验收阶段")
    selected = {case["id"] for case in catalog()["cases"]
                if STAGES.index(case["stage"]) <= STAGES.index(stage)}
    blockers = [{"id": case["id"], "status": case["status"], "reason": case["reason"]}
                for case in report["cases"] if case["id"] in selected and case["status"] != "PASS"]
    return {"stage": stage, "required_cases": len(selected), "acceptance_passed": not blockers,
            "publication_authorized": False, "blockers": blockers}


def merge_reports(reports, root, subject):
    require(bool(reports), "没有待合并报告")
    merged = copy.deepcopy(validate_report(reports[0], root, subject))
    entries = {case["id"]: case for case in merged["cases"]}
    for report in reports[1:]:
        validate_report(report, root, subject)
        for case in report["cases"]:
            previous = entries[case["id"]]
            if case["status"] == "NOT_RUN" or previous == case:
                continue
            require(previous["status"] == "NOT_RUN", f"{case['id']}: 存在冲突结果，不能覆盖失败或旧证据")
            previous.clear()
            previous.update(copy.deepcopy(case))
    return validate_report(merged, root, subject)


def verify_source_catalog(subject):
    # 案例及支持矩阵都来自固定发布提交，不能临时降低某一端的合同。
    for relative in ("release/acceptance-cases.json", "release/product.toml"):
        try:
            content = subprocess.check_output(["git", "show", f"{subject['source_commit']}:{relative}"], cwd=ROOT, stderr=subprocess.DEVNULL)
        except subprocess.CalledProcessError as error:
            raise AcceptanceError("发布提交未包含当前验收目录或产品声明") from error
        require(hashlib.sha256(content).hexdigest() == digest(ROOT / relative), "验收目录或支持矩阵与固定发布提交不符")


def write_report(path, report):
    # 每次写新文件；显式选择新路径才能修订，保留先前失败证据。
    with Path(path).open("x", encoding="utf-8") as output:
        output.write(json.dumps(report, ensure_ascii=False, indent=2, allow_nan=False) + "\n")


def run_rust(subject, root):
    """仅执行固定仓库契约测试；不运行来自 manifest/报告的命令。"""
    head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    dirty = subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip()
    require(head == subject["source_commit"] and not dirty, "测试需使用固定发布提交的干净工作区")
    report = initial_report(subject)
    case = next(c for c in report["cases"] if c["id"] == "G1.rust")
    directory = Path(root) / ("rust-" + uuid4().hex)
    directory.mkdir(parents=True, exist_ok=False)
    commands = []
    evidence = []
    failed = 0
    skipped = 0
    blocked = False
    started_at = now()
    suites = (
        ("inputia-core", "sqlite-memory"), ("inputia-handy-runtime", None),
        ("inputia-capi", "bundled-static-rime,managed-memory"), ("inputia-rime", "bundled-static-rime"), ("inputia-settings", None),
        ("inputia-release", None), ("inputia-updater", "native-code-verification"),
    )
    for crate, features in suites:
        command = ["cargo", "test", "--locked", "--manifest-path", f"crates/{crate}/Cargo.toml"]
        if features:
            command += ["--features", features]
        # 动态 Rime 测试的“环境缺失”历史分支会正常 return；必须收集输出并计为跳过。
        command += ["--", "--nocapture"]
        commands.append(" ".join(command))
        try:
            result = subprocess.run(command, cwd=ROOT, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                    text=True, timeout=900, check=False)
            content = result.stdout
            summaries = re.findall(r"test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored", content)
            passed = sum(int(s[0]) for s in summaries)
            skipped += sum(int(s[2]) for s in summaries)
            skipped += len(re.findall(r"(?im)^\s*skip(?:ped)?[ :：]", content))
            failed += int(result.returncode != 0 or passed == 0)
        except (OSError, subprocess.TimeoutExpired) as error:
            content = f"固定测试命令未完成：{type(error).__name__}\n"
            blocked = True
        log = directory / (crate + ".log")
        log.write_text(content, encoding="utf-8")
        evidence.append({"path": log.relative_to(root).as_posix(), "sha256": digest(log)})
    # 测试期间源码发生变化会使证据失效，不能归属于原提交。
    after = subprocess.check_output(["git", "status", "--porcelain"], cwd=ROOT, text=True).strip()
    after_head = subprocess.check_output(["git", "rev-parse", "HEAD"], cwd=ROOT, text=True).strip()
    blocked |= bool(after) or after_head != head
    case.update(status="BLOCKED" if blocked else "FAIL" if failed or skipped else "PASS",
                evidence_level="integration", reason="固定 Rust 契约测试；不代表原生、签名或完整发布验收",
                procedure=commands, started_at=started_at, executed_at=now(),
                machine={"os": "macos" if sys.platform == "darwin" else platform.system().lower(),
                         "os_version": platform.mac_ver()[0] if sys.platform == "darwin" else platform.release(),
                         "architecture": platform.machine(), "model": "not-applicable"},
                evidence=evidence, metrics={"failed_assertions": failed, "skipped_required": skipped})
    execution = directory / "execution.json"
    write_report(execution, execution_payload(case, subject, {"kind": "builtin", "identity": "inputia-rust-contracts/v2", "reviewer": None}))
    case["execution_record"] = {"path": execution.relative_to(root).as_posix(), "sha256": digest(execution)}
    return validate_report(report, root, subject)


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="command", required=True)
    initialize = sub.add_parser("init", help="生成完整 NOT_RUN 矩阵，不执行应用")
    initialize.add_argument("--manifest", required=True, type=Path)
    initialize.add_argument("--artifact-root", type=Path,
                            help="最终分发制品根目录；提供后逐文件核对 manifest 摘要")
    initialize.add_argument("--output", required=True, type=Path)
    run = sub.add_parser("run-rust", help="在固定干净提交运行 Rust 契约并生成完整账本")
    run.add_argument("--manifest", required=True, type=Path)
    run.add_argument("--artifact-root", required=True, type=Path,
                     help="最终分发制品根目录；逐文件核对 manifest 摘要")
    run.add_argument("--evidence-root", required=True, type=Path)
    run.add_argument("--output", required=True, type=Path)
    for command in ("verify", "merge"):
        item = sub.add_parser(command)
        item.add_argument("--manifest", required=True, type=Path)
        item.add_argument("--artifact-root", required=True, type=Path,
                          help="最终分发制品根目录；逐文件核对 manifest 摘要")
        item.add_argument("--report", required=True, action="append", type=Path)
        item.add_argument("--evidence-root", required=True, type=Path)
        if command == "merge":
            item.add_argument("--output", required=True, type=Path)
        else:
            item.add_argument("--stage", choices=STAGES, default="pre-public")
    args = parser.parse_args(argv)
    try:
        subject = manifest_subject(args.manifest, args.artifact_root)
        verify_source_catalog(subject)
        if args.command == "init":
            write_report(args.output, initial_report(subject))
            print(json.dumps({"initialized": True, "acceptance_passed": False}))
            return 0
        if args.command == "run-rust":
            report = run_rust(subject, args.evidence_root.resolve())
            write_report(args.output, report)
            result = summarize(report, "pre-public")
            print(json.dumps(result, ensure_ascii=False, indent=2))
            return 0 if result["acceptance_passed"] else 2
        report = merge_reports([read_json(p) for p in args.report], args.evidence_root, subject)
        if args.command == "merge":
            write_report(args.output, report)
            print(json.dumps({"merged": True, "publication_authorized": False}))
            return 0
        result = summarize(report, args.stage)
        print(json.dumps(result, ensure_ascii=False, indent=2))
        return 0 if result["acceptance_passed"] else 2
    except (ValueError, OSError, KeyError, TypeError) as error:
        print(json.dumps({"acceptance_passed": False, "error": str(error)}, ensure_ascii=False), file=sys.stderr)
        return 1


if __name__ == "__main__":
    sys.exit(main())
