#ifndef INPUTIA_UNIFIED_PAIR_AUTH_BRIDGE_H
#define INPUTIA_UNIFIED_PAIR_AUTH_BRIDGE_H
#include <stddef.h>
#include <stdint.h>

/* 返回码：0 成功，1 参数无效，2 manifest 拒绝，3 对端/硬化拒绝。 */
typedef struct UipaVerifiedPeer {
    uint8_t audit_token[32];
    uint32_t uid;
    uint32_t role; /* 1 Handy，2 Inputia */
} UipaVerifiedPeer;

/* 信任参数只能来自签名前嵌入的构建常量；无关闭硬化的参数。 */
int32_t uipa_manifest_load(
    const uint8_t *envelope, size_t envelope_len,
    const uint8_t *public_key, size_t public_key_len,
    const uint8_t *key_id, size_t key_id_len,
    const uint8_t *run_id, size_t run_id_len,
    const uint8_t *profile_id, size_t profile_id_len,
    uint32_t local_role, void **out_manifest);
/* v2 只绑定编译期产品/发布；不接收 profile、渠道或安装收据公钥。失败不回退 v1。 */
int32_t uipa_manifest_load_v2(
    const uint8_t *envelope, size_t envelope_len,
    const uint8_t *public_key, size_t public_key_len,
    const uint8_t *key_id, size_t key_id_len,
    const uint8_t *product_id, size_t product_id_len,
    const uint8_t *release_id, size_t release_id_len,
    uint32_t protocol_major, uint32_t local_role, void **out_manifest);
/* fd 不转移所有权。manifest 必须为 load 创建且尚未释放的 handle。 */
int32_t uipa_authenticate(void *manifest, int32_t fd, uint32_t expected_role,
                          UipaVerifiedPeer *out_peer);
/* v2 必须从已验证的本机收据传入准确组件路径；禁止通过旧入口跳过路径验证。 */
int32_t uipa_authenticate_v2(void *manifest, int32_t fd, uint32_t expected_role,
                            const uint8_t *expected_path, size_t expected_path_len,
                            UipaVerifiedPeer *out_peer);
/* 仅释放一次；禁止与 authenticate 并发。NULL 为无操作。 */
void uipa_manifest_free(void *manifest);
#endif
