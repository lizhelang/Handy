# 配对验证固定夹具

`v1-frozen-manifest.fixture` 是在加入 v2 之前，用旧版 `PairAuthTool` 生成的签名清单原始字节。不要格式化或重新签名此文件。它与 `v1-frozen-public-key.hex` 用于证明现有 v1 验证行为保持兼容，并确认 v2 不会降级接受旧清单。

该公钥来自一次性、独立测试密钥。临时私钥已删除，没有使用 Keychain 或 Apple 证书；清单中两个程序的 CDHash 是合成值，不能用于产品程序认证。
