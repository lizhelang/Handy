#!/usr/bin/env python3
"""抽取真实权限恢复与监控函数，使用合成原生边界进行实际时间回归。"""

import argparse
import hashlib
from pathlib import Path
import subprocess
import sys
import tempfile
import time


ROOT = Path(__file__).resolve().parents[1]
parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--source", type=Path, help="指定旧 input_permission.rs，进行行为失败对照")
args = parser.parse_args()
if sys.platform != "darwin":
    parser.error("此测试必须在 macOS 运行，避免跳过生产监控分支")
source_path = args.source or ROOT / "src-tauri/src/input_permission.rs"
source = source_path.read_text()
print(f"source={source_path} sha256={hashlib.sha256(source.encode()).hexdigest()}", flush=True)


def declaration(marker: str) -> str:
    start = source.index(marker)
    brace = source.index("{", start)
    depth, end = 1, brace + 1
    while depth and end < len(source):
        depth += (source[end] == "{") - (source[end] == "}")
        end += 1
    if depth:
        raise ValueError(f"声明不完整：{marker}")
    return source[start:end]


# 保留同一版本的常量、票据、Lifecycle、请求准入函数，不补造旧源不存在的 API。
prefix = source.index("const PROBE_INTERVAL_MS:") if "const PROBE_INTERVAL_MS:" in source else source.index("struct Lifecycle {")
methods = source[prefix:source.index("fn root(")]
for marker in [
    "fn retire(", "fn retirement_health(", "fn launch(",
    "pub fn request_initialize(", "pub fn request_recheck(", "pub fn start_monitor(",
]:
    methods += "\n" + declaration(marker)
if "fn background_state(" in source:
    methods += "\n" + declaration("fn background_state(")
    health = 'let state = s.state.lock().unwrap(); background_state(s, &state)'
else:
    # 旧版本没有分类 helper，抽取其 publish_health 中原有的完整分类表达式。
    publish = declaration("fn publish_health(")
    health = publish[publish.index("    let st ="):publish.index("    let value =")] + "\nstate"

classifier_test = r'''
#[test]
fn native_failure_classification() -> Result<(), String> {
    for reason in ["快捷键原生监听器意外退出", "Carbon 注册或注销回执未知",
        "旧原生键盘线程尚未退出", "input_device_busy",
        "Failed to initialize Enigo: no connection could be established: (failed creating event source)"] {
        ensure(recovery_reason(reason).is_some(), &format!("已知原生故障未归类：{reason}"))?;
    }
    for reason in ["permission_required", "maintenance_marker", "Shortcut cannot be empty",
        "shortcut_settings_retirement_unconfirmed", "input_device_poisoned", "synthetic_unknown_error",
        "Failed to initialize Enigo: the application does not have the permission to simulate input"] {
        ensure(recovery_reason(reason).is_none(), &format!("不应自动恢复的错误被接受：{reason}"))?;
    }
    println!("classification known_native=5 denied_config_unknown=7 passed=true");
    Ok(())
}
''' if "fn recovery_reason(" in source else ""

fixture = r'''
#![allow(dead_code, unused_variables, private_interfaces)]
extern crate self as log;
#[macro_export] macro_rules! warn { ($($arg:tt)*) => {{}}; }
#[macro_export] macro_rules! info { ($($arg:tt)*) => {{}}; }
#[macro_export] macro_rules! debug { ($($arg:tt)*) => {{}}; }
use std::sync::{atomic::{AtomicBool, AtomicU64, Ordering}, Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Clone)]
pub struct AppHandle;
impl AppHandle {
    fn manage<T>(&self, _: T) {}
    fn try_state<T: 'static>(&self) -> Option<&'static T> { None }
}
struct TranscriptionCoordinator;
impl TranscriptionCoordinator { fn request_cancel(&self) {} }
mod commands { pub struct ShortcutsInitialized; }

static PROBES: AtomicU64 = AtomicU64::new(0);
static PROBE_FINISHED: AtomicU64 = AtomicU64::new(0);
static PROBE_DELAY_MS: AtomicU64 = AtomicU64::new(0);
static BLOCK_FIRST_PROBE: AtomicBool = AtomicBool::new(false);
static RELEASE_FIRST_PROBE: AtomicBool = AtomicBool::new(false);
static LAST_OBSERVED_AT: AtomicU64 = AtomicU64::new(0);
static PROBE_STARTS: Mutex<Vec<u64>> = Mutex::new(Vec::new());
static RETIRE_KEYS: AtomicU64 = AtomicU64::new(0);
static RETIRE_INPUT: AtomicU64 = AtomicU64::new(0);
static RETIRE_TARGETS: AtomicU64 = AtomicU64::new(0);
static INITIALIZE_INPUT: AtomicU64 = AtomicU64::new(0);
static INITIALIZE_KEYS: AtomicU64 = AtomicU64::new(0);
static RETIRE_DELAY_MS: AtomicU64 = AtomicU64::new(0);
static INITIALIZE_DELAY_MS: AtomicU64 = AtomicU64::new(0);
static NATIVE_ACTIVE: AtomicU64 = AtomicU64::new(0);
static NATIVE_MAX_ACTIVE: AtomicU64 = AtomicU64::new(0);
static LAST_NATIVE_FINISHED_AT: AtomicU64 = AtomicU64::new(0);
static NATIVE_STEPS: Mutex<Vec<(&'static str, u64)>> = Mutex::new(Vec::new());
static AUDIT_CLOSED_INITIALIZATION: AtomicBool = AtomicBool::new(false);
static ADMISSION_AUDIT_SAMPLES: AtomicU64 = AtomicU64::new(0);
static EARLY_ADMISSION: AtomicU64 = AtomicU64::new(0);

fn audit_closed_admission() {
    if !AUDIT_CLOSED_INITIALIZATION.load(Ordering::SeqCst)
        || PROBE_FINISHED.load(Ordering::SeqCst) >= 3 { return; }
    let s = lifecycle();
    let allowed = s.gate.load(Ordering::SeqCst) || s.check(s.epoch.load(Ordering::SeqCst)).is_ok();
    // 再读完成计数，排除采样恰跨最终 probe 完成时刻的误报。
    if allowed && PROBE_FINISHED.load(Ordering::SeqCst) < 3 {
        EARLY_ADMISSION.fetch_add(1, Ordering::SeqCst);
    }
    ADMISSION_AUDIT_SAMPLES.fetch_add(1, Ordering::SeqCst);
}

fn native_step(label: &'static str, delay: u64) {
    audit_closed_admission();
    let active = NATIVE_ACTIVE.fetch_add(1, Ordering::SeqCst) + 1;
    NATIVE_MAX_ACTIVE.fetch_max(active, Ordering::SeqCst);
    let start = Instant::now();
    std::thread::sleep(Duration::from_millis(delay));
    NATIVE_STEPS.lock().unwrap().push((label, start.elapsed().as_millis() as u64));
    LAST_NATIVE_FINISHED_AT.store(monotonic_ms(), Ordering::SeqCst);
    NATIVE_ACTIVE.fetch_sub(1, Ordering::SeqCst);
    audit_closed_admission();
}
fn native_permission(epoch: u64) -> Result<(), String> {
    if initializing_epoch()? == epoch { Ok(()) } else { Err("stale_input_initialization".into()) }
}
mod shortcut {
    use super::*;
    pub fn retire_shortcuts(_: &AppHandle) -> Result<(), String> {
        RETIRE_KEYS.fetch_add(1, Ordering::SeqCst);
        native_step("retire_shortcuts", RETIRE_DELAY_MS.load(Ordering::SeqCst));
        Ok(())
    }
    pub fn init_shortcuts(_: &AppHandle) -> Result<(), String> {
        let epoch = initializing_epoch()?;
        INITIALIZE_KEYS.fetch_add(1, Ordering::SeqCst);
        native_step("initialize_shortcuts", INITIALIZE_DELAY_MS.load(Ordering::SeqCst));
        native_permission(epoch)
    }
    pub fn health_ready(_: &AppHandle, epoch: u64) -> Result<(), String> { native_permission(epoch) }
}
mod input {
    use super::*;
    pub fn retire_enigo(_: &AppHandle) -> Result<(), String> {
        RETIRE_INPUT.fetch_add(1, Ordering::SeqCst); Ok(())
    }
    pub fn initialize_enigo(_: &AppHandle, epoch: u64) -> Result<(), String> {
        native_permission(epoch)?;
        INITIALIZE_INPUT.fetch_add(1, Ordering::SeqCst);
        native_step("initialize_input", INITIALIZE_DELAY_MS.load(Ordering::SeqCst));
        native_permission(epoch)
    }
}
mod ime_target_broker {
    use super::*;
    pub fn invalidate_all(_: &AppHandle) -> Result<(), String> {
        RETIRE_TARGETS.fetch_add(1, Ordering::SeqCst); Ok(())
    }
}
fn marker_active(_: &AppHandle) -> bool { false }
fn probe() -> (bool, bool) {
    audit_closed_admission();
    let index = PROBES.fetch_add(1, Ordering::SeqCst);
    PROBE_STARTS.lock().unwrap().push(monotonic_ms());
    if index == 0 && BLOCK_FIRST_PROBE.load(Ordering::SeqCst) {
        while !RELEASE_FIRST_PROBE.load(Ordering::SeqCst) {
            std::thread::sleep(Duration::from_millis(2));
        }
    }
    std::thread::sleep(Duration::from_millis(PROBE_DELAY_MS.load(Ordering::SeqCst)));
    audit_closed_admission();
    LAST_OBSERVED_AT.store(monotonic_ms(), Ordering::SeqCst);
    PROBE_FINISHED.fetch_add(1, Ordering::SeqCst);
    (true, true)
}
fn health_label(s: &Lifecycle) -> &'static str { __HEALTH__ }
// 仅替换健康文件写入；状态分类仍使用源文件原有表达式。
fn publish_health(_: &AppHandle) { let _ = health_label(lifecycle()); }
__METHODS__

fn ensure(value: bool, reason: &str) -> Result<(), String> {
    if value { Ok(()) } else { Err(reason.into()) }
}
fn wait_for(timeout: Duration, condition: impl Fn() -> bool, reason: &str) -> Result<(), String> {
    let deadline = Instant::now() + timeout;
    while !condition() {
        audit_closed_admission();
        if Instant::now() >= deadline { return Err(reason.into()); }
        std::thread::sleep(Duration::from_millis(5));
    }
    Ok(())
}
// 每个场景独立进程；这里仅安排进入场景之前的已就绪状态，不复制生产状态转换。
fn seed_ready(lease_ms: u64) {
    let s = lifecycle();
    s.invalidated.store(false, Ordering::SeqCst);
    s.gate.store(true, Ordering::SeqCst);
    s.authorized.store(true, Ordering::SeqCst);
    s.ready_epoch.store(1, Ordering::SeqCst);
    s.authorized_epoch.store(1, Ordering::SeqCst);
    s.lease_until.store(monotonic_ms() + lease_ms, Ordering::SeqCst);
    let mut state = s.state.lock().unwrap();
    state.accessibility = "granted"; state.input_monitoring = "granted";
    state.enigo = "ready"; state.shortcuts = "ready"; state.targets = "ready";
}
fn report(label: &str) {
    let s = lifecycle();
    println!("scenario={label} epoch={} gate={} busy={} recovery={} health={} probes={} finished={} retire_keys={} init_input={} init_keys={} error={:?}",
        s.epoch.load(Ordering::SeqCst), s.gate.load(Ordering::SeqCst), s.busy.load(Ordering::SeqCst),
        s.lease_recovery_allowed(), health_label(s), PROBES.load(Ordering::SeqCst),
        PROBE_FINISHED.load(Ordering::SeqCst), RETIRE_KEYS.load(Ordering::SeqCst),
        INITIALIZE_INPUT.load(Ordering::SeqCst), INITIALIZE_KEYS.load(Ordering::SeqCst),
        s.state.lock().unwrap().error);
}

#[test]
fn stale_probe_preserves_new_recovery_and_reaches_ready() -> Result<(), String> {
    seed_ready(300);
    BLOCK_FIRST_PROBE.store(true, Ordering::SeqCst);
    launch(&AppHandle, false);
    wait_for(Duration::from_secs(1), || PROBES.load(Ordering::SeqCst) == 1, "旧 probe 未启动")?;
    start_monitor(&AppHandle);
    wait_for(Duration::from_secs(1), || lifecycle().epoch.load(Ordering::SeqCst) == 2,
        "真实 200ms 监控未将到期租约推进到 epoch 2")?;
    ensure(lifecycle().lease_recovery_allowed(), "租约到期没有安排恢复")?;
    RELEASE_FIRST_PROBE.store(true, Ordering::SeqCst);
    wait_for(Duration::from_secs(1), || !lifecycle().busy.load(Ordering::SeqCst), "旧 probe 未结束")?;
    report("stale_probe_finished");
    let recovery_preserved = lifecycle().lease_recovery_allowed();
    let ready = wait_for(Duration::from_secs(5), || lifecycle().gate.load(Ordering::SeqCst),
        "后续真实 probe/native 初始化未恢复 ready");
    report("stale_probe_recovery_result");
    ensure(recovery_preserved, "旧 probe 失败清除了 epoch 2 的新恢复计划")?;
    ready?;
    ensure(INITIALIZE_INPUT.load(Ordering::SeqCst) == 1 && INITIALIZE_KEYS.load(Ordering::SeqCst) == 1,
        "恢复没有恰好完成一套真实初始化调用")?;
    ensure(lifecycle().epoch.load(Ordering::SeqCst) == 2, "旧 probe 改写了后来的恢复代数")
}

#[test]
fn slow_native_steps_finish_with_permission_verified() -> Result<(), String> {
    seed_ready(3500);
    close_gate("permission_required");
    RETIRE_DELAY_MS.store(2000, Ordering::SeqCst);
    INITIALIZE_DELAY_MS.store(2000, Ordering::SeqCst);
    PROBE_DELAY_MS.store(30, Ordering::SeqCst);
    AUDIT_CLOSED_INITIALIZATION.store(true, Ordering::SeqCst);
    launch(&AppHandle, true);
    start_monitor(&AppHandle);
    let audit_until = Instant::now() + Duration::from_millis(1700);
    while Instant::now() < audit_until {
        audit_closed_admission();
        std::thread::sleep(Duration::from_millis(5));
    }
    let still_busy = lifecycle().busy.load(Ordering::SeqCst);
    let gate_closed = !lifecycle().gate.load(Ordering::SeqCst);
    let started = Instant::now();
    let result = wait_for(Duration::from_secs(10), || lifecycle().gate.load(Ordering::SeqCst),
        "每步 2s 的退休/构造过程未恢复 ready");
    report("slow_native_result");
    println!("native_steps={:?} remaining_wait_ms={} max_active={}", NATIVE_STEPS.lock().unwrap(),
        started.elapsed().as_millis(), NATIVE_MAX_ACTIVE.load(Ordering::SeqCst));
    println!("admission_audit_samples={} early_gate_or_check_success={}",
        ADMISSION_AUDIT_SAMPLES.load(Ordering::SeqCst), EARLY_ADMISSION.load(Ordering::SeqCst));
    ensure(still_busy && gate_closed, "软超时应保留 native 单飞且禁止业务输入")?;
    result?;
    ensure(EARLY_ADMISSION.load(Ordering::SeqCst) == 0
        && ADMISSION_AUDIT_SAMPLES.load(Ordering::SeqCst) > 100,
        "退休开始至最终 probe 完成前，gate/check 提前允许业务输入")?;
    let steps = NATIVE_STEPS.lock().unwrap();
    for expected in ["retire_shortcuts", "initialize_input", "initialize_shortcuts"] {
        ensure(steps.iter().any(|(name, elapsed)| *name == expected && *elapsed >= 1900),
            &format!("没有真实执行 2s 步骤：{expected}"))?;
    }
    ensure(NATIVE_MAX_ACTIVE.load(Ordering::SeqCst) == 1, "原生构造出现重叠 owner")?;
    ensure(PROBE_FINISHED.load(Ordering::SeqCst) >= 3, "退休后和初始化后没有重新 probe")?;
    let observed = LAST_OBSERVED_AT.load(Ordering::SeqCst);
    let last_native = LAST_NATIVE_FINISHED_AT.load(Ordering::SeqCst);
    let deadline = lifecycle().lease_until.load(Ordering::SeqCst);
    println!("final_probe_observed_ms={observed} native_finished_ms={last_native} public_deadline_ms={deadline}");
    ensure(observed > last_native, "最终权限 probe 没有发生在 native 初始化完成之后")?;
    ensure(deadline >= observed && deadline - observed <= 3500, "公开租约超出最终 probe 的 3.5s")
}

#[test]
fn ready_two_second_probes_keep_original_public_lease_bound() -> Result<(), String> {
    seed_ready(3500);
    PROBE_DELAY_MS.store(2000, Ordering::SeqCst);
    let original_deadline = lifecycle().lease_until.load(Ordering::SeqCst);
    start_monitor(&AppHandle);
    wait_for(Duration::from_secs(1), || PROBES.load(Ordering::SeqCst) >= 1, "监控未启动 probe")?;
    std::thread::sleep(Duration::from_millis(1000));
    let in_flight_did_not_extend = lifecycle().lease_until.load(Ordering::SeqCst) == original_deadline;
    let until = Instant::now() + Duration::from_millis(5500);
    let mut gate_drops = 0;
    let mut max_public_proof = 0;
    while Instant::now() < until {
        if !lifecycle().gate.load(Ordering::SeqCst) { gate_drops += 1; }
        max_public_proof = max_public_proof.max(proof_valid_for_ms(10_000));
        std::thread::sleep(Duration::from_millis(20));
    }
    report("ready_two_second_probes");
    println!("probe_starts_ms={:?} gate_drops={} max_public_proof_ms={} in_flight_deadline_unchanged={}",
        PROBE_STARTS.lock().unwrap(), gate_drops, max_public_proof, in_flight_did_not_extend);
    ensure(in_flight_did_not_extend, "在途 probe 偷延长了旧公开租约")?;
    ensure(gate_drops == 0 && lifecycle().epoch.load(Ordering::SeqCst) == 1,
        "稳定 2s 的真实 probe 被软超时或错误限频反复关闭")?;
    ensure(PROBE_FINISHED.load(Ordering::SeqCst) >= 3, "启动时刻限频没有持续完成至少三轮 probe")?;
    ensure(max_public_proof <= 3500, "公开证明超过 3.5s")?;
    ensure(INITIALIZE_INPUT.load(Ordering::SeqCst) == 0 && RETIRE_KEYS.load(Ordering::SeqCst) == 0,
        "健康 probe 触发不必要的退休/重建")
}

#[test]
fn stopped_without_recovery_does_not_repeat_native_work() -> Result<(), String> {
    seed_ready(3500);
    close_gate("synthetic_nonrecoverable");
    retire(&AppHandle)?;
    let before = (PROBES.load(Ordering::SeqCst), RETIRE_KEYS.load(Ordering::SeqCst),
        RETIRE_INPUT.load(Ordering::SeqCst), RETIRE_TARGETS.load(Ordering::SeqCst));
    start_monitor(&AppHandle);
    std::thread::sleep(Duration::from_millis(4400));
    report("stopped_without_recovery");
    let after = (PROBES.load(Ordering::SeqCst), RETIRE_KEYS.load(Ordering::SeqCst),
        RETIRE_INPUT.load(Ordering::SeqCst), RETIRE_TARGETS.load(Ordering::SeqCst));
    println!("stopped_native_counts_before={before:?} after={after:?}");
    ensure(before == after, "无恢复计划且已 stopped，监控仍循环 probe/retire")?;
    ensure(health_label(lifecycle()) == "paused", "已经 stopped 却仍报告 retiring")?;
    ensure(!lifecycle().gate.load(Ordering::SeqCst), "明确关闭被后台监控擅自重开")
}
__CLASSIFIER_TEST__
'''
fixture = fixture.replace("__METHODS__", methods).replace("__HEALTH__", health).replace("__CLASSIFIER_TEST__", classifier_test)
scenarios = [
    "stale_probe_preserves_new_recovery_and_reaches_ready",
    "slow_native_steps_finish_with_permission_verified",
    "ready_two_second_probes_keep_original_public_lease_bound",
    "stopped_without_recovery_does_not_repeat_native_work",
]
if classifier_test:
    scenarios.append("native_failure_classification")
else:
    print("SKIP native_failure_classification：旧源码不存在分类 API；通用行为场景照常运行", flush=True)

with tempfile.TemporaryDirectory(prefix="inputia-permission-recovery-") as temporary:
    directory = Path(temporary)
    rust = directory / "self_check.rs"
    binary = directory / "self-check"
    rust.write_text(fixture)
    compiled = subprocess.run([
        "rustup", "run", "1.96.0", "rustc", "--edition=2021", "--test", str(rust), "-o", str(binary),
    ], check=False)
    if compiled.returncode:
        print("fixtureCompilationFailed=true behavioralRegressionEvidence=false", flush=True)
        raise SystemExit(2)
    failed = 0
    for scenario in scenarios:
        started = time.monotonic()
        try:
            result = subprocess.run([str(binary), "--exact", scenario, "--nocapture", "--test-threads=1"],
                check=False, timeout=20)
            passed = result.returncode == 0
        except subprocess.TimeoutExpired:
            passed = False
            print(f"场景超过 20s：{scenario}", flush=True)
        failed += not passed
        print(f"scenario={scenario} passed={str(passed).lower()} elapsed_s={time.monotonic() - started:.2f}", flush=True)
    print(f"permissionRecoverySelfCheck={str(failed == 0).lower()} scenarios={len(scenarios)} failures={failed} "
        "productionLifecycle=true productionMonitor=true syntheticNativeBoundary=true nativePermissionAcceptance=false", flush=True)
    raise SystemExit(1 if failed else 0)
