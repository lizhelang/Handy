# schema4兼容恢复构建准备

工作区 `/Users/lzl/FILE/github/Handy-unified-input-compat`，分支 `codex/unified-input-compat`，以当前已安装控制中心代码 `9fa26dd3` 为底座。不是重置主实施分支；日常与候选安装均未改动。

仅回移已审查的SourceOutbox schema4及app备份升级门槛，保持新来源列/投影并接受schema4，不降低数据库版本；旧用户界面、语音和剪贴板能力保留，不带入新逐词确认UI。3处测试fixture因新增列改为显式原字段列表。

验证：source_outbox14通过、app库473通过2忽略、前端build/lint通过。日志 `/tmp/inputia-compat-source-tests.log`、`/tmp/inputia-compat-app-tests.log`、`/tmp/inputia-compat-frontend.log`。Qwen helper从原构建资源复制（相同旧代码），不重新选择模型/外部依赖；bun锁文件安装未改变依赖版本。

构建复用 `/Users/lzl/FILE/github/Handy-unified-input-system/src-tauri/target` 缓存，只允许顺序运行；新的主候选已单独保留于 `provenance-candidate-20260909.ZfTOxZ`，不依赖会被后续构建覆盖的bundle输出。恢复包最终应独立保存并配对签署。

尚未证明：新版本写入新增记录/确认/遗忘后换此构建的实际启动与对账、旧Host导入不复活、原生语音闭环。尤其旧导入重复学习缺陷仍存在，不能将“支持schema4”宣称为完整A12或可日常发布。
