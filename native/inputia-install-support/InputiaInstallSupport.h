#ifndef INPUTIA_INSTALL_SUPPORT_H
#define INPUTIA_INSTALL_SUPPORT_H
#include <stddef.h>
#include <stdint.h>

/* 输入必须指向 length 个有效字节，且 0 < length <= 32768。
 * 结果为独立 JSON 字符串，用 iuis_string_free 释放；NULL 表示失败。
 * ABI 仅验当前用户拥有的受管制品代码，不授权安装或停止进程，不读取 Keychain。
 * 调用方须固定 staging、防止并发改写；结果不提供整树原子锁，修改或恢复后须重验。 */
char *iuis_verify_code(const uint8_t *bytes, size_t length);
void iuis_string_free(char *string);
#endif
