# 发布元数据与构建入口

`product.toml` 是 Inputia 发布版本、构建号、产品与组件身份、目标支持矩阵和渠道路径的唯一源。当前 `1.1.0 / 84` 是现有本机基线。目标系统列表表示拟支持范围，具体系统通过情况必须来自最终制品验收。

## 可直接使用的命令

开发构建工具要求 **Python 3.11 或更新版本**，只使用标准库。macOS 自带 `/usr/bin/python3` 可能仍为 3.9；它会明确报错。可用 `INPUTIA_RELEASE_PYTHON=/绝对路径/python3` 选择解释器。此依赖不属于最终安装器要求。

```sh
python3 scripts/inputia_release.py validate-product
python3 scripts/inputia_release.py generate-config --output-dir "$PWD"
python3 scripts/inputia_release.py check-config
scripts/build-inputia-release.sh --preflight
scripts/build-inputia-release.sh --preflight-public
python3 -m unittest discover -s scripts/tests -p test_inputia_release.py -v
```

公共预检可接收受控流水线生成的绑定证据：

```sh
python3 scripts/inputia_release.py preflight --mode public \
  --public-evidence /绝对路径/public-release-evidence.json
```

证据文件为 schema v2，必须引用真实的 signed manifest、受信公钥集、冻结制品根目录、验收报告、验收证据根目录和待核验的公证 DMG/PKG。预检会重新计算文件摘要、调用 manifest 信封验签、逐文件核对制品，重新校验每个 `PASS` 案例的 execution record 摘要/主体/结果绑定，并要求所有 pre-public 案例为 `PASS`；组件签名由 manifest/组件检查负责，分发归档在 macOS 上执行 `spctl --assess --type open`。缺少文件、提交不一致或任一项失败都会阻断。

`generate-config` 只写指定输出目录下的三个受管文件：

- `src-tauri/tauri.inputia-release.conf.json`
- `src-tauri/InputiaReleaseInfo.plist`
- `release/generated/build-metadata.json`

版本变动先修改 `product.toml`，再生成配置。`check-config` 忽略排版差异，拒绝字段值、额外字段或文件缺失造成的漂移。

默认构建脚本现在只读预检，并拒绝不在构建矩阵内的宿主系统/架构。已有本机配对构建使用 `scripts/build-inputia-release.sh --build-local`，必须显式提供原有签名身份和配对输入。它先检查元数据，再创建唯一 `release_id` 和实读 Git commit 的 `build-context.json`，向控制中心、IME、设置入口统一注入版本与构建身份；签名前应用 plist，构建后重新读取三端实际 plist，并通过 `lipo`/`otool` 检查各主可执行文件的架构与最低系统。该检查不表示所有嵌套库、签名和公证已通过。当前仍是本机 v1 profile 信任兼容入口，输出会明确标记 `public_release_eligible=false`。

`prepare --output-dir <新的规范化绝对目录>` 可以独立生成本次构建输入，重复目录会被拒绝。每次调用都分配新 release ID，即使 version/build/commit 相同也不复用身份。工作树是否干净记录在上下文中；公共模式拒绝脏工作树。

`--build-local-v2` 现在必须显式提供 `INPUTIA_UPDATER_APP` 和 `INPUTIA_BOOTSTRAP_APP`。两个路径必须指向已构建、非符号链接的独立 `.app`；脚本会把它们复制到冻结目录并按正式五组件范围检查。缺少任一组件时构建直接失败，不会再生成只有三组件却带 v2 配对身份的包。

## 四种文档各自负责什么

| 文档                       | 内容                                                           | 不能放入的内容                                                     |
| -------------------------- | -------------------------------------------------------------- | ------------------------------------------------------------------ |
| `build-context.json`       | 本次构建身份、真实提交、工作树状态、目标和阶段                 | 未执行的验收成功声明                                               |
| `release-manifest.json`    | 不可变组件归档、最终分发文件摘要、配对摘要、逐库兼容、回滚目标 | channel、profile、installation ID、验收/attestation 摘要、自身摘要 |
| `release-attestation.json` | manifest 字节摘要、验收报告摘要、逐回滚目标实测报告摘要        | 修改制品内容或反向注入 manifest                                    |
| `channel-feed.json`        | 渠道、轨道 sequence、有效期和 manifest/attestation 指针        | 作为已安装离线输入的运行依赖                                       |

候选晋级稳定渠道时，两个 feed 引用相同 manifest 摘要。更改程序或签名、公证票据意味着新制品和新 release ID。渠道和本机 profile 不写入公开 release manifest；当前本机 v1 的 profile 字段仅在临时构建 overlay 注入，并继续阻断公共发布。

## 结构验证与摘要验证

```sh
python3 scripts/inputia_release.py validate \
  --kind manifest --document /绝对路径/release-manifest.json \
  --artifact-dir /绝对路径/冻结制品目录
```

若文档是 signed envelope，可额外提供独立保存的受信公钥集进行密码校验：

```sh
python3 scripts/inputia_release.py validate \
  --kind manifest --document /绝对路径/signed-manifest.json \
  --trusted-keys /绝对路径/trusted-keys.json
```

公钥集格式为 `{\"threshold\":1,\"keys\":[{\"key_id\":\"sha256-...\",\"public_key_x963_base64\":\"...\"}]}`。公钥 ID 必须是未压缩 P-256 X9.63 公钥原始字节的 SHA-256；工具通过系统 `openssl` 校验签名，信封中的 key ID 本身不会建立信任。未提供公钥集时报告 `NOT_RUN`，提供后只有达到阈值才报告 `PASS`。

冻结制品后可用 `bind-manifest` 将模板绑定到本次 `build-context.json`，由工具重新计算组件、分发包和配对清单的摘要/大小：

```sh
python3 scripts/inputia_release.py bind-manifest \
  --template /绝对路径/manifest-template.json \
  --context /绝对路径/build-context.json \
  --artifact-dir /绝对路径/冻结制品目录 \
  --output /绝对路径/release-manifest.json
```

该命令拒绝上下文身份、目标或产品摘要不一致，拒绝软链接和目录制品，并以独占方式写出未签名清单；它不会生成签名、公证或公开发布授权。

`--kind` 还支持 `attestation`、`feed`。输入可以是原始 payload，或符合 `signed-envelope.schema.json` 的 `{schema_version,payload_kind,payload,signatures}`。返回的 `document_sha256` 始终覆盖输入文件的原始字节。签名待签编码使用 `canonical_bytes(payload)`：UTF-8、排序字段、无多余空格、仅 schema 允许的有界整数；`signatures` 不进入待签字节。

- 结构检查拒绝重复 JSON 字段、NaN/Infinity、未知字段、布尔值假整数、重复身份、越界范围、绝对路径和路径穿越。
- 每个数据库必须声明可读/可写 schema、可读/写事件格式、outbox、修订和删除/遗忘能力。回滚目标必须能读写当前数据、双向理解事件并保留当前能力，不能只声明“可读取”。这些是待实测的合同，不自动构成兼容证明。
- `components[].sha256` 覆盖冻结的普通归档文件；`.app` 目录不能被当成一个普通文件计算摘要。代码 CDHash 与 Developer ID requirement 另存。`distribution_artifacts` 覆盖最终 DMG/安装器归档。
- `--artifact-dir` 对实际普通文件核对摘要、大小与目录边界，拒绝 symlink。它不会解包归档，也不表示已检查内部归档路径或可执行签名。
- 当前签名校验只对显式提供的受信公钥集执行；未提供时返回 `signature_verification=NOT_RUN`，签名无效或阈值不足会失败。`public_release_eligible` 始终为 false。

Python 调用方可使用 `load_product()`、`read_json(path)`、`validate_manifest(payload, product)`、`unwrap_document(value, kind, product)`、`verify_artifacts(payload, directory)`；合同错误抛 `ReleaseError`（`ValueError` 子类）。`validate_manifest` 默认允许重验历史版本；显式传入 `expect_current_build=True` 才要求匹配当前 `product.toml` 的版本与构建目标。

## 当前公共预检的明确阻断项

公共入口会拒绝：配置漂移、脏提交、仍绑定 profile 的 v1 信任、缺少真实证据包、未经验证的 Developer ID/公证与最终制品验收。手动把 `public_release_enabled` 改为 true 不能绕过这些门槛。

此目录已实现元数据与预检合同。v2 配对认证与安装定位已接入源码；默认 `--build-local` 仍保留明确的 v1 本机桥接入口。发布签名/根密钥轮换、签名安装器、持久恢复更新核心和真实设备验收继续实施，不能将本机脚本测试计为这些门槛通过。

## v2 配对与本机安装收据

`native/unified-pair-auth/build_trust.py --release-context <build-context.json>` 生成 v2 公开构建元数据和 Rust/Swift 编译常量；不能同时提供 `--run-id`。它核对本次真实源码提交，并绑定构建上下文摘要。v2 配对签名使用独立签名域，只包含产品、release、key、协议与组件代码身份；v1 冻结夹具和独立入口继续保留。v2 验签或安装绑定失败不会自动调用 v1。

两端通过 `inputia-settings::installation` 共用定位合同；Swift 在创建输入法会话前调用无 session 的 C ABI。安装收据固定在当前用户 `Library/Application Support/Inputia/installation.json`，要求所有者正确、权限私有、无符号链接或硬链接。读取有大小限制，路径逐段通过目录句柄打开。

收据字段为 `schema_version=1`、`product_id`、随机且稳定的 `installation_id` / `profile_id`、内核 `uid`、`scope`、`data`、`components`、当前 `release_id` 和渠道偏好 `channel`：

- `scope=user` 的组件路径固定为当前用户 Applications 下控制中心/设置入口，以及 Library/Input Methods 下 IME；`legacy_single_user` 明确保留 `/Applications/Inputia.app`。
- `data.kind=managed` 使用 `Inputia/Profiles/<profile_id>/Handy` 与 `Inputia`；`legacy_candidate` 保留 `HandyUnifiedCandidate/<run_id>/Handy` 与 `Inputia`，profile 必须仍是 `unified-candidate:<run_id>`。不移动已有数据。
- 配对文件定位到 `Inputia/Releases/<release_id>/pair-manifest.json`；receipt 不能指定任意替代路径。当前包路径与对端代码路径都要匹配收据。
- v2 握手 minor 1 必须声明 `installation_binding_v1`，双方核对 product、installation、profile 与 release；v1 minor 0 仅用于双方均明确选择旧合同的桥接。

收据由后续安装事务创建/切换。此批代码没有为日用安装生成收据、迁移证书或替换程序；真实 `.app` 路径、旧版混配和桥接升级仍需原计划的实机验收。路径比对是动态代码认证之外的附加约束，不承诺消除同 UID 文件替换的所有竞态。

## 原生发布目录信任

`crates/inputia-release` 提供签名域、根链双阈值轮换、不可变历史归档策略、feed v2、逐库升级/回滚元数据校验和每用户耐久高水位。公开元数据授权入口统一经过持久 `TrustStore`；状态缺失/损坏不重置，频道切换不清空序号，已观察过期的时间不会因系统时钟回拨而失效。

产品元数据已固定 updater 和 bootstrap 的 Bundle ID。旧 feed v1 仍可被开发工具作结构检查，原生新更新入口只接受 v2；不存在验签失败回退 v1。Python `validate` 的显式受信公钥验签已接入，但正式构建、签署、下载及更新适配器的接线仍在后续实施中。

完整格式、耐久顺序、测试范围和未完成边界见 [`inputia-release/README.md`](../crates/inputia-release/README.md)。当前新增代码不启用公共发布，也没有读取日用应用或真实密钥。

## 验收账本与执行入口

`acceptance-cases.json` 定义已批准计划的 27 个必需案例。验收工具从固定发布提交核对案例目录与支持矩阵，报告绑定 manifest 原始字节摘要、源码提交和最终分发制品摘要。

```sh
python3 scripts/verify-inputia-release init \
  --manifest /绝对路径/release-manifest.json --output /绝对路径/acceptance.json
python3 scripts/verify-inputia-release run-rust \
  --manifest /绝对路径/release-manifest.json \
  --evidence-root /绝对路径/证据目录 --output /绝对路径/rust-acceptance.json
python3 scripts/verify-inputia-release merge \
  --manifest /绝对路径/release-manifest.json \
  --report /绝对路径/rust-acceptance.json --report /绝对路径/native-acceptance.json \
  --evidence-root /绝对路径/证据目录 --output /绝对路径/merged-acceptance.json
python3 scripts/verify-inputia-release verify \
  --manifest /绝对路径/release-manifest.json --report /绝对路径/merged-acceptance.json \
  --evidence-root /绝对路径/证据目录 --stage pre-public
```

`init` 只生成完整 `NOT_RUN` 矩阵。`run-rust` 仅调用固定的 Core/Runtime/CAPI/Settings 测试命令，不执行清单或报告里的任意命令；需要固定提交的干净工作区，证据目录应放在工作区之外。即使这组测试通过，其他必需案例仍未运行，因此完整门禁返回 2。格式/绑定错误返回 1，通过相应阶段才返回 0；`publication_authorized` 始终为 false，发布授权与签名证明另行检查。

每个 `PASS` 除附件摘要外，还必须引用独立 `execution_record` JSON 文件：`schema_version`、完整 `subject`、`producer` 和 `result`。`result` 原样绑定案例 ID、状态、证据等级、步骤、开始/结束 UTC 时间、机器、指标及附件摘要。内置执行器只可证明其固定范围；外部/手工记录使用 `producer.kind=reviewed`，包含执行者和不同的复核者标识。复核声明与最终签名证明分别处理，本地格式检查不声称能识别伪造的人工体验证据。

合并仅填充 `NOT_RUN`，冲突的已执行结果不得被静默覆盖；重跑应保留旧文件并形成新报告。文件路径、证据摘要、平台、样本、指标合法范围、候选下载→至少 7 天观察→稳定下载时序均校验。`pre-public`、`candidate`、`stable` 阶段逐步增加必需项；少数成功案例、UI mock、跳过原生检查、不适用或受阻都不能替代完整通过。旧 `post-install-regression.sh` 在 UI smoke 未运行时返回 `BLOCKED`/退出码 8，非 GUI 系统检查通过单独记录。
