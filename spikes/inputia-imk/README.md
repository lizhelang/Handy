# Inputia InputMethodKit Spike

这个 spike 只验证 macOS 系统级输入法 Host 的最小可行边界：

- Swift 可以链接 `InputMethodKit`。
- `.app` bundle 可以携带输入法注册所需的 `Info.plist` 键。
- `IMKServer` 可以按 `InputMethodConnectionName` 启动。
- `IMKInputController` 子类可以接收 key event，并把普通英文字符提交给客户端。
- 用户级安装后，TIS 能否注册、枚举、启用和选择输入源。

当前 spike 不做：

- 拼音、双拼、候选窗、记忆层或设置页。

运行：

```bash
./spikes/inputia-imk/build.sh
./spikes/inputia-imk/install-user.sh
```

产物：

```text
spikes/inputia-imk/build/InputiaIMKSpike.app
```

当前结论见 `EVIDENCE.md`：bundle 能注册和枚举，但在当前用户级自制安装条件下仍不能进入 TIS enabled source list，因此 `TISSelectInputSource` 返回 `-50`。继续验证需要系统级安装、系统设置交互或正式签名路径。
