# Inputia 发布目录信任核心

原生 Rust 库，不要求安装机具备 Python、源码或编译器。当前完成根轮换、目录验签、频道新鲜度、逐库元数据兼容与耐久防重放，并接入主应用的受限 HTTPS 目录查询；**原生安装适配器、离线新装授权和正式签署流水线仍待完成，不能据此宣称产品可发布。**

## 签名与摘要合同

- ECDSA P-256 / SHA-256，X9.63 非压缩公钥，DER 签名，严格标准 Base64。key ID 固定为公钥字节的 `sha256-<hex>`，不能用别名凑阈值。
- 签名原文为 `Inputia.Release.v1\0` + UTF-8 用途名 + `\0` + canonical payload。用途独立于组件配对签名；用途包括 manifest、attestation、feed、keyset。保留的 recovery/offline_install 标签尚无安装授权入口。
- Canonical JSON 明确递归排序，UTF-8 不做 Unicode 归一化，保留非 ASCII 字符，只接受 ±(2^53−1) 以内整数；拒绝重复键、浮点/指数/负零、尾随内容与超限文档。即使上层启用 `serde_json/preserve_order`，签名字节也保持一致。
- `keyset_digest` 和频道序号对应的 `document_digest` 是上述带域签名原文的 SHA-256。空白、签名顺序、合法新增签名或 ECDSA 签名字节变化不产生链分叉。
- manifest、attestation、验收/回滚报告和实际制品仍按**完整文件字节**计算摘要。频道引用的固定清单不因 candidate→stable 改写或重签。

## 根链与历史制品

`TrustRoot::from_embedded` 只能接编译期产品根；下载的 keyset 不能作为自签信任起点。每次只接受连续 N→N+1 和准确前驱语义摘要，并同时满足旧、新 root 阈值。keyset 最多 32 个签名，可覆盖两套各 16 个 root 键；其他文档最多 16 个签名。

旧根过期仍可验证下一根，以便离线较久的安装逐级恢复；最终用于新更新的 keyset 必须当前有效。root 与在线角色的键用途分离。撤销集合只能增加；撤销不能靠随后更新 keyset 复活。

每个 keyset 保留不可改写、不可移除的 `archive_policies`：它们冻结当年的 manifest/attestation 角色和阈值。当前角色从一签变成二签不会破坏旧制品验证。Retired 键只验证历史制品，不能签发新 feed；最新撤销始终覆盖历史策略。当前有效 feed 必须选择历史策略并精确授权目标 manifest/attestation 摘要。归档数学验签自身不产生安装许可。

## 新频道合同

`channel-feed-v2.schema.json` 增加 keyset 版本/语义摘要、归档策略 ID 与显式 rollback record；有效期最多七天。按 `(product, channel, platform, architecture)` 分轨道保存最高序号；同序号同正文幂等，不同正文拒绝。频道不能切换清零，也不能因版本更旧自动降级。

`AuthorizedReleaseMetadata` 只能由 `TrustStore` 产出。生产调用链为：

```text
内置根 → open / 重验已存连续根链 → advance_keyset
→ commit_feed（先持久化观察时间和防重放高水位）
→ authorize_release_metadata
→ 后续逐制品下载校验 / Apple 验签 / 原生事务预检
```

元数据授权校验：产品/发布/目标一致；报告与原始文件摘要绑定；回滚报告集合准确；本机架构、已测试系统大版本、最低系统与 updater 版本符合；逐库读写范围、事件格式、隐私/修订/outbox 能力可承接当前版本。未知迁移要求明确拒绝。降级必须匹配已装源版本及摘要、源版本签名 attestation 认可的准确报告、目标制品摘要与逐库合同。

该类型只证明目录声明获授权，**不证明报告中的真实测试已经执行**。原生安装仍须验证解包文件和 Apple 代码身份、实际库状态、独立恢复环境及发布门禁。签署流水线必须先运行严格验收执行器；客户端不能把一个摘要匹配的任意报告当作业务验收通过。

授权完成时同时冻结 `NativeReleasePolicy`：五个组件的角色、bundle ID、Team、归档摘要/大小及逐 slice CDHash，还有配对清单 schema、signer key ID 和摘要，都从同一份已验签 manifest 复制为字段私有、不可反序列化的强类型对象。后续读取裸 manifest 或内部合法 JSON 变化不会改写该策略。当前公开目标固定 arm64，因此每个组件必须恰好声明一个 20 字节 Apple CDHash；多值或 64 位摘要在 manifest 语义层拒绝，不能由 updater 任选或截断。

`verify_artifact_files` 对清单列出的组件归档、分发包和配对清单进行有界流式 SHA-256 校验，核对完整文件大小、总容量预算、所有者和权限，拒绝文件/祖先软链接、多硬链接、重复 inode 与读取期间变化。结果仅描述这次从文件描述符读取的字节；实际暂存副本仍要重验，并执行原生代码签名与解包检查。

## 耐久状态

固定每用户目录 `~/Library/Application Support/Inputia/UpdateTrust/`：

- `state.lock`：进程级非阻塞排他锁。
- `state.json`：根链、全部频道高水位、已观察最大系统时间；临时文件 fsync → 原子替换 → 目录 fsync 后才返回成功。
- `initialized.json`：初始化完成标记。之后状态缺失或损坏进入修复错误，不能悄悄重置历史。
- `keyset-<语义摘要>.json`：原始签名 keyset，先完整写临时文件后原子不覆盖发布；指针提交前崩溃只留下可重用的完整孤立文件。部分临时文件永不当作根链。

逐目录 fd 打开，拒绝软链接、可疑归属/权限、非普通文件、硬链接与超限大小；账本文件 0600，根目录 0700。读取重新验证整条签名根链。写失败使句柄失效。已观察过 feed 过期的系统时间也先落盘，回拨系统时钟不会让该 feed 重新有效。

这里不声称抵御同 UID 攻击者整体替换完整信任目录，或首次使用前提供错误系统时间；独立离线根恢复包与完整本地回滚威胁防护仍须在后续恢复方案中明确处理。

## 验证

```sh
cargo test --manifest-path crates/inputia-release/Cargo.toml --offline --features serde_json/preserve_order
python3.11 -m unittest discover -s scripts/tests
```

当前本 crate 25 项测试通过；发布/验收 Python 测试数量以实施记录的最新整套回归为准。覆盖真实临时密钥签名、不同根阈值、归档策略保留、域隔离、等价包装、逐库反例、报告错绑、降级、原生策略冻结、文件锁互斥、重启、时钟回拨、状态缺失/损坏/链接和部分文件，以及落盘归档内容、总预算和链接反例。使用合成制品和临时目录，未访问真实发布证书、日用安装或私人数据。

## 更新目录查询接线

`catalog::check_catalog` 接受构建内置信任、固定来源适配器、系统时钟，以及当前安装的签名清单基线。依次下载频道、最多 32 个连续根轮换、准确摘要的清单/证明/报告，最终重新打开耐久信任存储完成授权。网络等待不占用文件锁；任何并发频道变化或失效根都会在最终检查被拒绝。响应单文档上限 4 MiB，总计 32 MiB，回滚报告最多 32 项；超出明确失败，不部分通过。长期离线需要更多根时，保存已验证进度并返回继续刷新状态。

固定公开路径：

- `keysets/<version>.json`：连续签名 keyset。
- `channels/<candidate|stable>/macos/arm64.json`：签名 feed v2。
- `releases/<release_id>/release-manifest.json`、`release-attestation.json`：原始签名封套。
- `releases/<release_id>/reports/<sha256>.json`：原始验收/回滚报告。

主应用内置 `release/update-source.json`；`source=null` 表示发布来源尚未配置。正式 `source` 包含 product_id、base_url、redirect_origins、root_keys、root_threshold。根公钥是发布时选定的产品信任，不能由设置、环境或下载响应注入。HTTPS 适配器仅跟随内置允许来源、最多三次重定向，不发送历史/文本/安装 ID；每个请求限时和流式计数，拒绝压缩正文及任意路径跳转。

旧安装必须有固定 `Inputia/Releases/<release_id>/release-manifest.json` 和私有 `catalog-receipt.json`（schema_version=1、release_id、archive_policy_id、manifest_digest）。旧清单仍须用当前根与历史归档策略验签，并匹配编译发布身份；缺失时明确要求修复，不伪装首次安装绕过逐库兼容。**这些文件还需由配套安装器以耐久事务写入，本包未修改日用安装。**

菜单与界面只调用 `check_product_update`。已移除 Tauri 单应用 updater 插件、ACL 与未验签安装链接回退。查询区分当前/发现版本/不可检查/禁用，进程内重复查询有界等待，关闭后迟到结果被丢弃；每次重新读取频道，等待后核对维护状态与完整安装收据。`installable` 当前固定 false，后续必须接入原生配套预检、下载暂存和恢复事务，不能凭查询结果直接安装。普通入口暂不授权降级，独立回退流程必须提供旧版签名报告。
