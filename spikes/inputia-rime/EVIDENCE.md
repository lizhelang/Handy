# Inputia librime Spike 证据

日期：2026-07-06

## 本机环境

- librime dylib：`/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib`
- Rime shared data：`/Library/Input Methods/Squirrel.app/Contents/SharedSupport`
- C API 头文件：`/tmp/inputia-librime/src/rime_api.h`

Homebrew 状态：

```text
brew info librime => Not installed
```

因此本 spike 使用已安装 Squirrel bundle 自带的成熟 librime runtime。

## 构建

命令：

```bash
git clone --depth 1 https://github.com/rime/librime.git /tmp/inputia-librime
./spikes/inputia-rime/build.sh
```

结果：

```text
/Users/lzl/FILE/github/Handy/spikes/inputia-rime/build/rime_probe
```

结论：可以用 librime 官方 C API 头文件链接 Squirrel bundle 内的 `librime.1.dylib`。

## 全拼候选

命令：

```bash
./spikes/inputia-rime/build/rime_probe luna_pinyin_simp ni
```

关键结果：

```text
schema=luna_pinyin_simp
keys=ni
preedit=ni
page=0 page_size=5 candidates=5 last=0 highlighted=0
candidate[0]=你
candidate[1]=拟
candidate[2]=尼
candidate[3]=泥
candidate[4]=呢
commit=<none>
```

命令：

```bash
./spikes/inputia-rime/build/rime_probe luna_pinyin_simp zhongguo
```

关键结果：

```text
schema=luna_pinyin_simp
keys=zhongguo
preedit=zhong guo
candidate[0]=中国
candidate[1]=种过
candidate[2]=种果
candidate[3]=忠果
candidate[4]=中古
```

结论：librime 可以作为第一版 `Chinese Engine Adapter` 的基础候选生成器。

## 上屏 commit

命令：

```bash
./spikes/inputia-rime/build/rime_probe luna_pinyin_simp 'ni{space}'
```

关键结果：

```text
schema=luna_pinyin_simp
keys=ni{space}
preedit=
commit=你
```

结论：librime 能从 key sequence 产生 commit text，Host/Core 需要把这个 commit 映射到平台 `insertText`。

## 已发现风险

- Squirrel 自带 schema 没有双拼 schema；已用 Rime 官方 `rime-double-pinyin` schema 在临时目录验证小鹤双拼。
- 并行 probe 共享同一个 `user_data_dir` 时出现 LevelDB lock；Inputia 必须保证 Rime userdb 单写入者策略，或把 Rime userdb 当作 adapter 内部缓存而非 Inputia Memory 真相来源。
- Rime 会写用户词典和 build 数据；Inputia Memory/Ranker 仍必须保留自己的本地事实库，不能把 Rime 用户词典当唯一记忆层。
- 当前 dylib 来自 Squirrel bundle，只能作为本机 spike 依据；正式打包要单独处理 librime 依赖、schema 数据、插件、许可证、签名和更新策略。

## 双拼候选

来源：

```bash
git clone --depth 1 https://github.com/rime/rime-double-pinyin.git /tmp/inputia-rime-double-pinyin
```

该仓库 README 列出自然码、智能 ABC、小鹤、微软、拼音加加、四通六种双拼方案，并声明依赖 `rime-luna-pinyin`。本 spike 使用 Squirrel bundle 的 `luna_pinyin` 和 `stroke` 数据，在临时 shared/user 目录中加入 `double_pinyin_flypy.schema.yaml`。

命令：

```bash
./spikes/inputia-rime/prepare-double-pinyin-data.sh double_pinyin_flypy
INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-rime-shared-double-pinyin \
INPUTIA_RIME_USER_DATA_DIR=/tmp/inputia-rime-user-double-pinyin \
  ./spikes/inputia-rime/build/rime_probe double_pinyin_flypy vsgo --option simplification=true
```

关键结果：

```text
option[simplification]=1
schema=double_pinyin_flypy
keys=vsgo
preedit=zhong guo
candidate[0]=中国
candidate[1]=种过
candidate[2]=种果
candidate[3]=忠果
candidate[4]=中
```

命令：

```bash
INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-rime-shared-double-pinyin \
INPUTIA_RIME_USER_DATA_DIR=/tmp/inputia-rime-user-double-pinyin \
  ./spikes/inputia-rime/build/rime_probe double_pinyin_flypy 'vsgo{space}' --option simplification=true
```

关键结果：

```text
commit=中国
```

结论：第一阶段可先支持小鹤双拼，设置层预留 schema id 以扩展自然码、微软等多方案。简体输出需要在 adapter 层显式设置 `simplification=true` 或对应 schema 默认值。

## 当前结论

第一版 Chinese Engine Adapter 可以先接 librime，但边界必须保持：

- Core 输入状态不暴露 librime session。
- Adapter 输出统一 `Candidate` 列表和 commit 事件。
- Ranker 在 Adapter 之后重排候选。
- Memory 不直接依赖 Rime userdb。
- 双拼 schema 已验证小鹤方案；正式实现仍要记录 schema 数据来源、许可证和打包策略。
