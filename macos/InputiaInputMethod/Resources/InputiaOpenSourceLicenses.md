# Inputia 开源许可说明

Inputia 第一阶段扩展词库只引入 MIT 许可数据源。未确认许可、GPL、CC BY-SA、GFDL、NC 或类似限制性数据源不在本轮引入范围内。

## 数据源

| 数据源         | 原始 URL                                         | 许可证 | 本项目中的用途                                            |
| -------------- | ------------------------------------------------ | ------ | --------------------------------------------------------- |
| THUOCL         | https://github.com/thunlp/THUOCL                 | MIT    | 常用成语、诗词固定片段和高频短语候选。                    |
| chinese-poetry | https://github.com/chinese-poetry/chinese-poetry | MIT    | 唐诗、宋词文本片段，用于生成整句、分句和 2-8 字内部片段。 |
| chinese-xinhua | https://github.com/pwxcoo/chinese-xinhua         | MIT    | 成语拼音；大字表导入保留为显式生成选项。                  |

## 转换与修改

- 将原始文本或 JSON 解析为 Rime `词条<TAB>拼音<TAB>权重` 格式。
- 拼音生成采用保守策略：优先使用原始数据提供的拼音；否则只在单字拼音唯一时按字拼合；多音字不确定的条目默认跳过。
- 为验收样例和少量常见多音词加入人工校正拼音，例如“长风破浪”“会有时”“千帆过”“烂柯人”。
- 罕见字第一阶段使用 Inputia 维护的轻量种子；`chinese-xinhua/data/word.json` 体积较大，生成器提供 `--include-xinhua-word` 显式选项，默认不在日常构建中拉取。
- 诗词数据会生成多层候选：整句、分句、2-8 字内部片段和高频固定搭配。
- 权重仅用于本地候选排序，不代表原数据源的官方频率解释。
- 生成产物不直接修改 Rime 原始 `luna_pinyin.dict.yaml`，而是通过 Inputia wrapper dictionary 导入原始词库和扩展词库。

## 免责声明

这些扩展词库按原许可证“按现状”提供。Inputia 会尽量保守处理拼音和多音字，但仍可能存在读音、切分、权重或候选排序不符合预期的情况。若发现错误，应优先修正生成脚本或添加可解释的本地校正规则，而不是手工改动生成后的词库文件。

## 候选字形字体

| 字体             | 原始 URL                               | 许可证            | 本项目中的用途                                                                          |
| ---------------- | -------------------------------------- | ----------------- | --------------------------------------------------------------------------------------- |
| Jigmo 2025-09-12 | https://kamichikoichi.github.io/jigmo/ | CC0 1.0 Universal | 为 macOS 系统字体缺失的合法 Unicode 生僻汉字提供候选窗字形，覆盖 CJK 基本区及扩展 A-J。 |

Inputia 对 Jigmo 基本区字体仅保留内置词库实际使用的字形；Jigmo2/Jigmo3 保留完整补充平面覆盖，因为 Rime/OpenCC 可能生成没有直接出现在源词库中的转换字符。字体内部名称改为 `Inputia Jigmo`、`Inputia Jigmo 2`、`Inputia Jigmo 3`，避免和用户自行安装的 Jigmo 冲突。生成脚本位于 `Tools/generate_candidate_fallback_fonts.py`，完整 CC0 文本随应用保存在 `Fonts/Jigmo-LICENSE.txt`。
