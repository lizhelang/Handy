# Inputia Rime Adapter 证据

日期：2026-07-06

## 设计边界

`crates/inputia-rime` 是 `Chinese Engine Adapter`，不是 Inputia Core：

- Core 只看到 `ChineseEngine` trait 和统一 `Candidate`。
- Adapter 内部动态加载 librime。
- Rime userdb 不是 Inputia Memory 的唯一事实来源。
- Ranker 仍在 Adapter 之后对候选重排。

## 证据来源

已核对：

- librime 官方 C API：`/tmp/inputia-librime/src/rime_api.h`
- 本机成熟实现：`/Library/Input Methods/Squirrel.app`
- spike：`spikes/inputia-rime`

当前 adapter 只绑定必要 API：

- `setup`
- `initialize`
- `start_maintenance`
- `create_session`
- `select_schema`
- `set_option`
- `simulate_key_sequence`
- `get_current_schema`
- `get_context`
- `get_commit`
- `destroy_session`
- `finalize`

## 验证命令

```bash
cargo test --manifest-path crates/inputia-rime/Cargo.toml
cargo test --manifest-path crates/inputia-rime/Cargo.toml --test core_flow -- --nocapture
cargo run --manifest-path crates/inputia-rime/Cargo.toml --example rime_probe -- luna_pinyin_simp ni
cargo run --manifest-path crates/inputia-rime/Cargo.toml --example core_flow_probe -- luna_pinyin_simp zhongguo 2
```

小鹤双拼：

```bash
./spikes/inputia-rime/prepare-double-pinyin-data.sh double_pinyin_flypy
INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-rime-shared-double-pinyin \
INPUTIA_RIME_USER_DATA_DIR=/tmp/inputia-rime-user-double-pinyin \
  cargo run --manifest-path crates/inputia-rime/Cargo.toml --example rime_probe -- double_pinyin_flypy vsgo
INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-rime-shared-double-pinyin \
INPUTIA_RIME_USER_DATA_DIR=/tmp/inputia-rime-user-double-pinyin \
  cargo run --manifest-path crates/inputia-rime/Cargo.toml --example core_flow_probe -- double_pinyin_flypy vsgo 2
```

## 验证结果

单元测试：

```text
running 1 test
test tests::squirrel_default_config_keeps_rime_outside_core ... ok

test result: ok. 1 passed; 0 failed
```

全拼：

```text
schema=luna_pinyin_simp
keys=ni
preedit=ni
page=0 page_size=5 candidates=5 last=false highlighted=0
candidate[0]=你
candidate[1]=拟
candidate[2]=尼
candidate[3]=泥
candidate[4]=呢
commit=<none>
```

小鹤双拼候选：

```text
schema=double_pinyin_flypy
keys=vsgo
preedit=zhong guo
page=0 page_size=5 candidates=5 last=false highlighted=0
candidate[0]=中国
candidate[1]=种过
candidate[2]=种果
candidate[3]=忠果
candidate[4]=中
commit=<none>
```

小鹤双拼上屏：

```text
schema=double_pinyin_flypy
keys=vsgo{space}
preedit=
page=0 page_size=0 candidates=0 last=false highlighted=0
commit=中国
```

Core + Rime 全拼输入流：

```text
mode=Chinese
after_input: composing=zhongguo page=0 visible_candidates=2
after_input: candidate[0]=中国
after_input: candidate[1]=种过
after_page_down: composing=zhongguo page=1 visible_candidates=2
after_page_down: candidate[0]=种果
after_page_down: candidate[1]=忠果
after_page_up: composing=zhongguo page=0 visible_candidates=2
after_page_up: candidate[0]=中国
after_page_up: candidate[1]=种过
commit=中国
after_commit: composing= page=0 visible_candidates=0
```

Core + Rime 小鹤双拼输入流：

```text
mode=Chinese
after_input: composing=vsgo page=0 visible_candidates=2
after_input: candidate[0]=中国
after_input: candidate[1]=种过
after_page_down: composing=vsgo page=1 visible_candidates=2
after_page_down: candidate[0]=种果
after_page_down: candidate[1]=忠果
after_page_up: composing=vsgo page=0 visible_candidates=2
after_page_up: candidate[0]=中国
after_page_up: candidate[1]=种过
commit=中国
after_commit: composing= page=0 visible_candidates=0
```

结论：

- Rust adapter 已能动态加载 Squirrel 自带 librime。
- `luna_pinyin_simp` 可产生全拼候选。
- `double_pinyin_flypy` 可产生小鹤双拼候选和 commit。
- `InputiaCore<RimeEngine>` 已验证全拼和小鹤双拼的候选显示、Core 分页、回到首页和空格上屏。
- 下一步是把这个 Core-flow 接入真实 IMK Host 的 key event 生命周期，并处理 Rime userdb 单写入者策略。
