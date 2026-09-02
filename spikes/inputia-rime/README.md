# Inputia librime Spike

这个 spike 验证 Inputia 的 `Chinese Engine Adapter` 可以把 librime 作为可替换候选生成器，而不是把 librime 当作 Inputia Core。

当前本机没有 Homebrew `librime` keg，但已安装的 Squirrel bundle 自带 `librime.1.dylib` 和 Rime schema 数据。因此本 spike 默认链接：

- `/Library/Input Methods/Squirrel.app/Contents/Frameworks/librime.1.dylib`
- `/Library/Input Methods/Squirrel.app/Contents/SharedSupport`

头文件来自 librime 官方源码 checkout：

```bash
git clone --depth 1 https://github.com/rime/librime.git /tmp/inputia-librime
```

构建和运行：

```bash
./spikes/inputia-rime/build.sh
./spikes/inputia-rime/build/rime_probe luna_pinyin_simp ni
```

验证小鹤双拼：

```bash
./spikes/inputia-rime/prepare-double-pinyin-data.sh double_pinyin_flypy
INPUTIA_RIME_SHARED_DATA_DIR=/tmp/inputia-rime-shared-double-pinyin \
INPUTIA_RIME_USER_DATA_DIR=/tmp/inputia-rime-user-double-pinyin \
  ./spikes/inputia-rime/build/rime_probe double_pinyin_flypy vsgo --option simplification=true
```

这个 spike 只验证 C API 初始化、部署、session、schema 选择、按键序列、preedit、候选和 commit。它不处理 Inputia 排序、隐私、用户记忆、候选窗 UI 或双拼 schema。
