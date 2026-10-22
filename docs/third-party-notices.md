# Third-Party Notices

## Ropy

Parts of Handy's clipboard file-path normalization and interaction design were adapted from [StudentWeis/ropy](https://github.com/StudentWeis/ropy), commit `1264415bf7d8746d5333f7a0405fecd15f466084`.

The distributable application also includes this notice at `Resources/resources/third-party-notices/ropy-LICENSE.txt`.

Copyright 2025-2026 The Ropy Authors

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and associated documentation files (the "Software"), to deal in the Software without restriction, including without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE SOFTWARE.

## Google Books Ngram Viewer dataset

The next-word prediction table `src-tauri/resources/personalization/prediction-ngram.sqlite` is derived from the Google Books Ngram Viewer dataset, version 3 (20200217), Chinese (Simplified) 1-grams and 2-grams, http://books.google.com/ngrams, licensed under the [Creative Commons Attribution 3.0 Unported License](https://creativecommons.org/licenses/by/3.0/).

Changes: counts restricted to 1980–2019, part-of-speech tagged tokens removed, both tokens limited to the bundled Rime public lexicon vocabulary, and the table pruned to the most frequent continuations per word. Source files, sizes and SHA-256 digests are pinned in `scripts/prediction-ngram.lock.json`; the transform is reproduced by `scripts/prepare-prediction-ngram.py`.
