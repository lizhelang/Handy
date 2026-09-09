# 设置调试日志的内容最小化

## 已观察到的风险

启动路径 load_or_create_app_settings 会以 Debug 输出 AppSettings。SecretMap 已保护 API 密钥，但原 derive(Debug) 仍可输出自定义词、提示词、设备名与路径。本次不把已有密钥脱敏错误描述成密钥已泄漏，也不声称审计了所有日志通道。

## 修改与不变项

AppSettings 的 Debug 改为只显示 settings_schema_version 和 [REDACTED]，不枚举其它配置。Serialize/Deserialize/Clone/Type 保持，用户实际配置保存、读取与迁移不变。两处设置解析错误只记录 serde 错误类别，不记录可能包含非法字段值的错误正文。

回归使用合成自定义词、提示词和设备名：原 Debug 会泄漏、测试先失败；修复后 Debug 不包含这些值，而序列化仍保留三者。原 API 密钥脱敏和设置迁移回归继续通过。

## 验证与交付边界

原生 Rust settings 模块26项测试通过，格式和diff检查通过；独立只读审查无阻塞，补充了设备名序列化保持断言。日志在 menu-20260909.wEdagq/settings-privacy-red.log、settings-privacy-final.log。

本批没有重启或更新现用候选，避免打断浮窗使用；本修复仅在源码和测试中验证，待下次候选打包带入。没有读取真实词库作为测试材料、修改实际设置、开启录音或播放音频。已有历史日志未删除，不把未来日志修复等同于过去诊断数据已擦除；完整 A07 和融合目标仍未最终验收。
