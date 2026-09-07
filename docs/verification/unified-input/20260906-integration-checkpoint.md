# 2026-09-06 接续检查点

北京时间2026-09-06 18:08，目标继续active；以下不是完整融合验收。

## 实际本地提交

- `d6935c66`：持久语音执行资格与恢复，runtime105项测试通过。
- `816ea261`：Rust/Swift实际配对认证桥及危险可执行页权限拒绝。
- `36d4b046`：原协调器明确会话、首帧准备、串行取消；音频失败仍保留成功文字。
- `b9d6b9e9`：Cancel注册资源序列、主线程fallback屏障及失败切换恢复。

`36d4b046`前后补足了真正生产保存路径的故障回归：实际WAV写入失败、验证失败、正常附件三种情况调用共用SQLite插入，重开验证原文/处理后文字/提示/附件身份。无附件删除不调用目录删除，重转写不读取目录。最近Handy全量424通过/2既有忽略；不是原生跨应用或完整隐私验收。

## 当前源码接线与测试

- `voice_dispatch.rs`已接实际Handy库，不再通过测试配置剥离真实认证构造和Coordinator适配。7项真实SQLite/注入测试通过：100个Start请求只一次调用、只读Status不增请求行、身份冒用拒绝、超时/断线仅查事实、明确Start拒绝持久Failed、Stop/Cancel拒绝不伪造终态、重启retired及撤销后本人关闭。
- 配对公钥构建步骤 `build_trust.py` 已生成Rust/Swift静态常量。6项合成结构测试通过；Handy实际带构建公钥的Clippy及测试通过。没有将运行时环境、设置、manifest当信任根。
- 本次待配对两端的公开构建元数据：`/private/tmp/handy-paired-build.BdoZjO/public-build.json`。离线签名私钥只在同一0700临时构建目录0600文件，仍需签最终两端manifest后删除，**不能打入候选包、提交或输出其内容**。原桥自检私钥与这次产品构建私钥不同，不能沿用“自检已清除”描述本文件。
- 新增Swift候选编译期profile核验；无配对构建的二进制不默认开放身份服务。实际服务socket仍未开启。

## 静态来源与资源

- 静态来源每次重新展开已校验归档到独立树；只有完整树匹配才复用受检generation。旧sources/deps/release缓存不被覆盖或自行晋升。
- `source_snapshot.py` 4项测试通过：新树复用、源/插件被改后保留旧树但重新生成、额外文件/改锁拒绝、越界链接/硬链接拒绝。
- 完整新树构建成功，日志 `/tmp/handy-static-source-rebuild-20260906.log`；构建前后完整树校验均通过，generation `fc81335c51f94b6dae1aa3a20d38b4bf`，tree digest `8cd7c92fbc1d9e9145e751135d111233bb9cf8344266ca9513c2090c05f6dd56`。
- 同一严格签名探针实际全拼、双拼、Lua filter及所有插件注册通过，日志 `native/static-rime/artifacts/probe-run.5q28wlbn/probe.log`。输出manifest绑定库、探针、头文件、源码锁及完整source-snapshot哈希。
- 直接重展开与build脚本权限掩码不同导致第一次未复用；差异仅8项目录/链接权限，源码字节一致。已统一脚本umask077，并重新执行重复展开核验；不把第一次reused=false称为成功复用。
- 统一权限后连续两轮新展开已实际确认第二轮 `reused=true`，generation `90ddf783f3194e64893b1d9567d5d597`，日志 `/tmp/handy-source-repeat-private-20260906.log`；既有库仍绑定其真正使用的fc81335来源，没有用新展开重新给旧库签发来源。
- 候选RimeData改为固定Squirrel官方包（仅展开、不安装）和固定schema归档；72项资源、原有共有业务资源逐字节一致、5份扩展词典未缩减。主代理重跑10项测试与verify-only通过。资源来源详见candidate-rime-data/README.md。
- 实际带公开配对常量的新Inputia候选构建完成，日志 `/tmp/inputia-paired-source-candidate-20260906.log`，严格签名和minOS13门禁通过；未安装，仍非最终提交产物。
- 新Host实际无窗口诊断通过，CDHash `88db0e193189f941e6f3961e7ba12d2b5070eda0`，嵌入key ID `candidate-05c3111a37497571c2398d1814438993aba5dbba9c3739b0f27ac53df9427b83`；runtime_key_configuration=false、真实全拼/双拼提交true、external_librime_loaded=false、imk_server_started=false、daily_user_dictionary_opened=false。

## 仍需实现，不能宣称完成

产品认证socket/握手audit绑定、策略与遗忘屏障、Host真实异步客户端与目标登记、owned结果持久交付/唯一上屏、按键路径异步学习仍未接通。现有owned ASR完成路径仍待替换旧paste后才能启用服务。

P2浮窗统一、P4真实确认术语识别对照及FunASR/Sherpa、P5控制中心、P6三应用原生/性能/完整兼容迁移回滚和最终安装包仍需完成。全部A01–A12维持未最终验收。没有触及日常安装、唯一真实数据、远程分支或正式发布。
