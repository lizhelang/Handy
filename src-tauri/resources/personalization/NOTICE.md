# 通用联想词频资料

原始词频来自 Rime 官方 rime-essay，通过固定的 Squirrel 1.1.2 发布包取得：

- 上游：https://github.com/rime/rime-essay
- 原始发布：https://github.com/rime/squirrel/releases/tag/1.1.2
- 许可：LGPL-3.0，见 LICENSE.rime-essay 及 COPYING.GPL-3.0。
- 未修改的原始资料：essay.original.txt。
- Inputia 派生文件：base-lexicon.tsv，使用已有OpenCC进行繁简转换，保留2至32字短语，合并简体同形词的词频。
- 具体校验值及转换记录：provenance.json；可用项目 scripts/prepare-personalization-lexicon.py 重建。

这是公共词典词频，用于冷启动的词汇补全和弱上下文增益；不是用户语料，不包含用户键入或剪贴板记录，也不代表完整语法模型。原始作者的著作权与许可保持不变。
