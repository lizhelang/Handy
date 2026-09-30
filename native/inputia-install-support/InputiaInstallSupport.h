#ifndef INPUTIA_INSTALL_SUPPORT_H
#define INPUTIA_INSTALL_SUPPORT_H
#include <stddef.h>
#include <stdint.h>

/* 输入必须指向 length 个有效字节，且 0 < length <= 32768。
 * 结果为独立 JSON 字符串，用 iuis_string_free 释放；NULL 表示失败。
 * ABI 仅验当前用户拥有的受管制品代码，不授权安装或停止进程，不读取 Keychain。
 * 调用方须固定 staging、防止并发改写；结果不提供整树原子锁，修改或恢复后须重验。 */
char *iuis_verify_code(const uint8_t *bytes, size_t length);
/* 存活期间暂停已核写者，不证明退出/数据库可交接。当前禁止生产安装接线：尚无crash guardian。
 * check/context仅同步借用，每个STOP前调用；handle由原生持有audit实例，不接受客户端PID。
 * suspend失败时handle为NULL；成功时必须由唯一拥有者最终free。接口不支持并发/复制handle。
 * 显式resume可重试且幂等，仅恢复本租约从运行态暂停的实例；free仅尽力恢复并释放。
 * 原本已经停止的进程不被本租约恢复。crash/abort/SIGKILL不会触发free。 */
typedef int32_t (*iuis_writer_effect_check)(void *context);
char *iuis_writer_suspend(const uint8_t *bytes, size_t length, iuis_writer_effect_check check,
                          void *context, void **handle);
char *iuis_writer_assert_suspended(void *handle, iuis_writer_effect_check check, void *context);
char *iuis_writer_resume(void *handle);
void iuis_writer_suspension_free(void *handle);
void iuis_string_free(char *string);
#endif
