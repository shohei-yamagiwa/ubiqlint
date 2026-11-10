# 変更履歴

## 未リリース（0.1.0）

ユビキタス言語の辞書に基づいて Java の命名とインポート境界を検査する、現在の開発版。
当面はソースからインストールして個人利用する。以下は現在の機能と制約の記録であり、
一般向けのリリース予定を定めるものではない。

### 機能

- `ubiqlint check`: 識別子の命名とインポート境界の検査。ファイル・ディレクトリの指定と、
  ソースに最も近い辞書の解決に対応する。
- `ubiqlint validate`: `ubiqlint.yaml` と `terms.yaml` の検証。
- `ubiqlint dictionary init`・MCP の `create_dictionary`: 既存のパスを上書きせず、
  明示したコンテキスト名で空の辞書を作成。
- `ubiqlint term`・`ubiqlint behavior`: 辞書の用語と振る舞いの追加・編集・削除。
- `ubiqlint mcp`: 辞書の取得・編集とコード検査を提供する stdio MCP サーバ。
- `ubiqlint lsp`: 未保存の Java バッファを検査し、辞書・設定の変更で再検査する
  診断専用の stdio LSP サーバ。
- Claude Code 向け LSP プラグインと Neovim 向けの設定例。

### 導入と制約

ソースからのインストール方法と最初の検査は [README](README.md)を参照。
ビルドには Rust 1.88 以上と C11 対応の C コンパイラ・リンカが必要。

- 対応言語は Java。LSP は補完・定義ジャンプ・コードアクションを提供しない。
- MCP・LSP の各サーバは 1 つの設定ファイルを扱う。
- Claude Code の同じ拡張子の複数 LSP には制約がある。
  [併用確認と回避策](docs/lsp.md#jdtls-lsp-との併用確認)を参照。
- 0.x の間はマイナーバージョンでも CLI・設定形式・診断・MCP ツールが
  互換性を保たずに変更される場合がある。
- 個人開発として、サポートはベストエフォート。
