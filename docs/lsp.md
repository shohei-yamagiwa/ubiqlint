# LSP で編集のたびに検査する

`ubiqlint lsp` は診断専用の LSP サーバです。Java の構文エラー、辞書に沿わない名前、インポート境界の違反を、エディタやエージェントに届けます。MCP の辞書取得・編集と組み合わせて使います。

## 1. 起動と検査の範囲

```sh
ubiqlint lsp
ubiqlint lsp --config path/to/ubiqlint.yaml
```

標準入出力で LSP（Content-Length 付きの JSON-RPC）を話します。通常のログやエラーは stderr に出します。

- 設定を省略すると、初期化時の `workspaceFolders` の最初のルート、なければ `rootUri`、`rootPath`、起動時のカレントディレクトリの順に選び、その下の `ubiqlint.yaml` を使います。扱う設定は 1 つです。複数のプロジェクトにはサーバを分けて起動してください
- `--config` は起動時のカレントディレクトリからのパスで、ワークスペースの指定より優先します。サーバはカレントディレクトリを変更しません
- 設定ファイルのディレクトリ以下の Java ファイルを検査します。`exclude`、ルールの `include`、最も近い辞書の解決は CLI・MCP と同じです。設定の範囲外、対応外の言語、除外されたファイルには診断を出しません
- `didOpen`・`didChange` で、未保存のバッファの全文を検査します。同期形式は全文（`Full`）で、差分同期には対応しません。`didClose` で診断を消します
- 設定と辞書はディスクから読みます。`terms.yaml` や使用中の設定ファイルの `didSave` を受けると、開いているファイルをすべて検査し直します。未保存の辞書・設定は検査に使いません
- ファイル監視の動的登録に対応するクライアントには、辞書と設定の作成・変更・削除を監視するよう要求します。MCP や別のプロセスが辞書を変更した場合も再検査します。監視に対応しないクライアントでは、次のソース編集・保存時に最新の辞書を読みます
- 設定・辞書が読めない、不正、または辞書がない場合は、検査できなかったソースの先頭に、問題のパスと理由を診断として出します。修正後の検査で消えます
- 命名違反の範囲は識別子全体、インポート違反は import 文全体です。`code` は設定のルール ID、`source` は `ubiqlint` です。構文エラーにはルール ID を付けません
- 位置は `utf-32` が利用できるクライアントでは UTF-32、それ以外は UTF-16 です。日本語、絵文字、Java の Unicode エスケープ、CRLF と単独 CR にも対応します

補完・定義ジャンプ・コードアクション・辞書の編集は提供しません。これらのリクエストには `MethodNotFound` を返します。

## 2. Claude Code

先に ubiqlint をインストールし、PATH を通します。

```sh
cargo install --path crates/ubiqlint-cli --locked
claude plugin marketplace add shohei-yamagiwa/ubiqlint
claude plugin install ubiqlint@ubiqlint
```

プラグインは [`plugins/ubiqlint`](../plugins/ubiqlint) にあります。`.lsp.json` は `.java` に対して `ubiqlint lsp` を起動します。ルート以外の設定を使う場合は、自分のプラグインの `args` に `--config` と絶対パスを指定してください。ローカルのプラグインは `claude --plugin-dir /absolute/path/to/plugins/ubiqlint` で試せます。

### jdtls-lsp との併用確認

**Claude Code 2.1.319 は、同じ `.java` の 2 つの LSP に編集通知を配信しません。** ubiqlint と jdtls の両方を登録するだけでは、両方の診断を受け取れません。

実装前に、npm の公式配布物 `@anthropic-ai/claude-code-linux-x64@2.1.319` に含まれる LSP 管理関数を取り出し、その処理を変更せず、サーバの生成だけを通知を記録するスタブに差し替えて試しました。`.java` を持つ `jdtls` と `ubiqlint` をこの順で登録し、同じファイルの open と change を呼びました。

```text
registered: jdtls, ubiqlint
didOpen:    jdtls のみ
didChange:  jdtls のみ
LSP: extension .java already handled by "jdtls"; "ubiqlint" will not be used for .java files
```

これは配布された管理処理の試験で、認証済みの Claude Code と実際の jdtls を使ったセッション全体の試験ではありません。2.1.0 の同じ管理処理でも通知先は先頭だけでした。将来の版で両方に配信されるようになった場合は、実際のセッションでも確かめてください。

ubiqlint の LSP を使う場合は、ほかの Java LSP を無効にします。公式の jdtls プラグインなら次のコマンドです。

```sh
claude plugin disable jdtls-lsp@claude-plugins-official
```

jdtls の定義ジャンプなどを使い続ける場合は、ubiqlint の LSP プラグインは有効にせず、[MCP または hooks](agents.md) で検査します。LSP プラグインが起動しない Claude Code のクラウドセッションでも MCP を使います。

## 3. Neovim

Neovim 0.11 以降では、Java のバッファで追加の LSP として起動できます。Neovim は 1 つのバッファに複数のサーバを接続できるため、jdtls と併用できます。

```lua
vim.api.nvim_create_autocmd("FileType", {
  pattern = "java",
  callback = function()
    local root = vim.fs.root(0, "ubiqlint.yaml")
    if not root then return end
    vim.lsp.start({
      name = "ubiqlint",
      cmd = { "ubiqlint", "lsp" },
      root_dir = root,
    })
  end,
})
```

ほかのエディタでも、追加の stdio LSP サーバを登録できるクライアントから同じコマンドを起動します。Java の拡張子・言語 ID と、ワークスペースのルートを指定してください。
