# 通用联想词频资料

原始词频来自 Rime 官方 rime-essay，通过固定的 Squirrel 1.1.2 发布包取得：

- 上游：https://github.com/rime/rime-essay
- 原始发布：https://github.com/rime/squirrel/releases/tag/1.1.2
- 许可：LGPL-3.0，见 LICENSE.rime-essay 及 COPYING.GPL-3.0。
- 未修改的原始资料：essay.original.txt。
- Inputia 派生文件：base-lexicon.tsv，使用已有OpenCC进行繁简转换，保留2至32字短语，合并简体同形词的词频。
- 具体校验值及转换记录：provenance.json；可用项目 scripts/prepare-personalization-lexicon.py 重建。

这是公共词典词频，用于冷启动的词汇补全和弱上下文增益；不是用户语料，不包含用户键入或剪贴板记录，也不代表完整语法模型。原始作者的著作权与许可保持不变。

# 上屏后联想二元表

prediction-ngram.sqlite 派生自谷歌图书 Ngram Viewer 数据集第三版（20200217）简体中文一元与二元文件：

- 来源：http://books.google.com/ngrams ，数据集说明 https://storage.googleapis.com/books/ngrams/books/datasetsv3.html
- 许可：Creative Commons Attribution 3.0 Unported（https://creativecommons.org/licenses/by/3.0/）。
- 改动：只保留 1980–2019 年计数，去掉带词性标记的词，两端都限定为上述 Rime 公共词表中的词，并按每个词保留最常见的续写。
- 原始文件大小与 SHA256 固定在项目 scripts/prediction-ngram.lock.json；转换记录见 prediction-ngram.provenance.json，可用 scripts/prepare-prediction-ngram.py 重建。

这是公开出版物的统计计数，不包含任何用户数据；原数据集的权利与许可保持不变。
