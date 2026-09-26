# Handy 历史迁移到 Inputia 验证记录

## 迁移范围

- 源数据：`~/Library/Application Support/com.pais.handy`
- 目标数据：`~/Library/Application Support/HandyUnifiedCandidate/trial-20260905/Handy`
- 完整备份：`~/Library/Application Support/HandyUnifiedBuilds/handy-migration-20260924-retry`
- 迁移脚本：`scripts/migrate-handy-history.py`

## 结果

- 旧语音记录 1,892 条全部写入目标 `history.db`；目标总数为 1,896 条（包含迁移前已有的 4 条）。
- 旧剪贴板记录 5,722 条中新增 5,606 条，116 条按 `content_hash` 去重；目标总数为 6,523 条（迁移期间产生了新的剪贴记录）。
- 1,895 个录音文件和 1,533 张图片均完成附件存在性校验，缺失数均为 0。
- 目标两个 SQLite 数据库的 `PRAGMA integrity_check` 均返回 `ok`。
- Inputia 历史页已现场打开旧语音，显示 6 秒录音控件；已现场打开迁移后的图片，显示图片预览。

## Handy 移除

为保留可恢复性，旧 Handy 没有直接永久擦除，而是从原位置移入备份目录：

- `/Applications/Handy.app`
- `~/Library/Application Support/com.pais.handy`
- `~/Library/Application Support/com.pais.handy.KnowledgePreview`
- 旧 Handy 偏好设置和 WebKit 数据

当前 `/Applications/Handy.app` 与旧 `com.pais.handy` 数据目录均不存在；Inputia Candidate 和输入法组件仍在运行，目标历史页可用。`/Users/lzl/FILE/github/Handy-unified-input-system` 源码目录没有删除。
