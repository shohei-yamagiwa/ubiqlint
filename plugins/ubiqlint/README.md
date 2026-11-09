# ubiqlint for Claude Code

Java の命名違反とインポート境界違反を LSP の診断として届けるプラグインです。`ubiqlint` を PATH にインストールしてから使います。設定ファイルは、Claude Code が渡すワークスペースのルートの `ubiqlint.yaml` です。

```sh
cargo install --path crates/ubiqlint-cli --locked
claude plugin marketplace add shohei-yamagiwa/ubiqlint
claude plugin install ubiqlint@ubiqlint
```

ローカルの変更を試す場合は、リポジトリを取得し、対象プロジェクトから次のように起動します。

```sh
claude --plugin-dir /absolute/path/to/ubiqlint/plugins/ubiqlint
```

**ほかの Java LSP との併用には制限があります。** Claude Code 2.1.319 の LSP 管理処理は、同じ拡張子の先頭のサーバにだけファイルの通知を送ります。`jdtls-lsp` と同時に登録しても、両方の診断は届きません。ubiqlint の LSP を使う場合は、ほかの Java LSP を無効にしてください。Java のコードナビゲーションを優先する場合は `jdtls-lsp` を残し、ubiqlint の検査には MCP の `check_files` または hooks を使います。

このプラグインは診断専用で、定義ジャンプや補完は提供しません。辞書の取得・編集には、別途 `ubiqlint mcp` を登録します。Claude Code のクラウドセッションではプラグインの LSP が起動しないため、MCP を使ってください。

併用確認の方法とエディタの設定は [LSP の文書](../../docs/lsp.md)、MCP と権限設定は [エージェントとの連携](../../docs/agents.md) を参照してください。
