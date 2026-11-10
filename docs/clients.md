# Desktop・Web アプリからクラウド上のコードを検査する

Windows から Claude Code、Codex、Mistral Vibe、VS Code を使い、
コードは GitHub から取得したクラウドの作業環境で編集する場合の手順です。
ubiqlint は、コードと辞書が置かれている実行環境にインストールします。
手元の Windows へのインストールだけでは、クラウド側の検査には使えません。

## 1. クラウド環境に CLI を導入する

シェルを実行できる作業環境では、まず CLI を使います。MCP・LSP の登録は必須ではありません。
Rust 1.88 以上、Cargo、Git、C11 対応の C コンパイラとリンカを用意してください。

クラウド環境に ubiqlint のソースが取得されている場合は、そのリポジトリのルートで
次を実行できます。公開前は、環境に取得するリポジトリとして ubiqlint も追加する方法が使えます。

```sh
cargo install --path crates/ubiqlint-cli --locked
ubiqlint --version
```

GitHub から直接導入する場合は、環境のセットアップ時に次を実行します。
初回の検証対象を固定するため、コミット SHA を指定しています。

```sh
cargo install --git https://github.com/shohei-yamagiwa/ubiqlint.git \
  --rev ab2ce7a4384ba30dabc9bd08400ea89b0c169446 --locked ubiqlint-cli
ubiqlint --version
```

正式リリース後は `--rev <SHA>` を `--tag <公開済みのタグ名>` に置き換えられます。
非公開の間は、クラウド環境にもこのリポジトリを取得する権限が必要です。
Cargo の取得だけが認証に失敗し、Git では取得できる場合は、環境設定の
`CARGO_NET_GIT_FETCH_WITH_CLI=true` で Git CLI を使えます。この設定は取得権限そのものを付与しません。

インストール先（通常は `$HOME/.cargo/bin`）を、**エージェントがコマンドを実行する際の PATH**
にも含めます。セットアップ中の `export` が次のセッションに引き継がれるかは環境ごとに確認してください。
ネットワークの許可もインストール時に必要です。導入後のコード検査にはネットワーク接続は不要です。

インストール後、検査するプロジェクトの `ubiqlint.yaml` があるディレクトリに移って実行します。

```sh
ubiqlint check
```

別のディレクトリから実行する場合は、クラウド内の設定ファイルを明示します。

```sh
ubiqlint check --config /absolute/path/to/project/ubiqlint.yaml
```

辞書・設定の作り方と正常・違反の例は [README](../README.md#最初の検査)を参照してください。

## 2. アプリごとの使い方

| アプリ・実行環境 | 最初に使う方法 | MCP・LSP を追加する場合 |
| --- | --- | --- |
| Claude Code のクラウドセッション | セットアップで CLI を導入し、エージェントから `ubiqlint check` を実行 | 単一リポジトリのセッションではプロジェクトの `.mcp.json` を使える。LSP プラグインのローカル設定はクラウドへ自動では引き継がれない |
| Codex のクラウド作業環境 | 環境の準備時に CLI を導入し、タスクから実行 | クラウド側で提供される MCP の登録方法・実行先を確認する。ローカル CLI 用設定を置くだけで登録完了とは扱わない |
| Mistral Vibe のコード実行環境 | シェルとリポジトリがある場合は CLI を導入して実行 | Vibe Code CLI の stdio 設定と、Vibe Work の URL を登録するコネクタは別の仕組み |
| クラウド環境に接続した VS Code | 接続先に CLI を導入し、リモートのターミナルやタスクから実行 | チャット用 MCP の設定と、エディタ診断用の LSP クライアント拡張は別に必要 |

### Claude Code

[公式のクラウド環境設定](https://code.claude.com/docs/en/cloud-environments)に沿って、
セットアップに CLI の導入を加えます。対象リポジトリの `CLAUDE.md` に検査コマンドを記載できます。

MCP も使う場合は、対象リポジトリのルートにある `.mcp.json` に次のサーバを追加します。
既存のサーバがある場合は、その設定を残して追加してください。
この例はセッションの作業ディレクトリが対象プロジェクトのルートであることを前提にしています。

```json
{
  "mcpServers": {
    "ubiqlint": {
      "type": "stdio",
      "command": "ubiqlint",
      "args": ["mcp", "--config", "ubiqlint.yaml"]
    }
  }
}
```

起動ディレクトリが異なる環境では `--config` にクラウド内の絶対パスを指定します。
次のセッションでサーバが接続され、`get_context`・`check_files` などが見えることを確認してください。
複数リポジトリのセッションなど、設定の読み込み条件は公式文書で確認します。
ツールの説明と辞書の扱いは[エージェントとの連携](agents.md)にあります。

### Codex

[公式のクラウド環境の手順](https://developers.openai.com/codex/environments/cloud-environments)に沿って、
再利用する環境の準備に CLI の導入を含めます。タスクから `ubiqlint --version` と検査を実行し、
新しいタスクでもインストール先と PATH が利用できることを確認してください。

対象リポジトリの `AGENTS.md` に、下の「エージェントへの指示例」を追加できます。
この方法なら、追加の MCP が登録されていない環境でもシェルから検査できます。

Codex CLI・IDE の stdio MCP 設定は[公式 MCP 文書](https://developers.openai.com/codex/mcp)にあります。
それを Windows 側に設定したことと、クラウドで動くタスクへの接続確認は分けて扱います。

### Mistral Vibe

コードを取得してシェルを実行できる環境では、同じ CLI の導入・検査手順を使います。
セットアップや初回の指示に導入手順と検査コマンドを記載してください。

クラウド側で Vibe Code CLI を実行しており、追加の stdio MCP を登録できる場合は、
その環境の `.vibe/config.toml` に次を追加できます。

```toml
[[mcp_servers]]
name = "ubiqlint"
transport = "stdio"
command = "ubiqlint"
args = ["mcp", "--config", "/absolute/path/to/project/ubiqlint.yaml"]
```

絶対パスはコードがあるクラウド内のパスに置き換えます。既存の `mcp_servers = []` がある場合は、
その空の定義を上の配列テーブルに置き換えます。プロジェクト設定の信頼・読み込み条件は
[公式の設定文書](https://docs.mistral.ai/vibe/code/cli/configuration)、
フィールドは[公式の MCP 文書](https://docs.mistral.ai/vibe/code/cli/mcp-servers)を参照してください。

Vibe Work の[カスタム MCP コネクタ](https://docs.mistral.ai/vibe/work/connectors/mcp-connectors)は
サーバ URL を登録する方式です。現在の `ubiqlint mcp` は stdio のみで、HTTP エンドポイントを
提供しないため、この設定や Windows 上の実行ファイルを URL 欄に登録することはできません。
シェルのないアプリで、URL のコネクタだけを通して使う構成は現行の配布対象に含めません。

### VS Code

コードがあるクラウド環境に接続し、そのターミナルで `ubiqlint --version` と
`ubiqlint check` が動くことを確認します。エディタだけが提供され、コマンドを実行する環境が
ない場合は、別の作業環境や CI で検査します。

VS Code のチャット用 MCP を使う場合は、対象ワークスペースの `.vscode/mcp.json` に追加します。

```json
{
  "servers": {
    "ubiqlint": {
      "type": "stdio",
      "command": "ubiqlint",
      "args": ["mcp", "--config", "${workspaceFolder}/ubiqlint.yaml"]
    }
  }
}
```

接続先の MCP 設定として登録されているか、サーバの起動先とログを確認してください。
コマンドパレットの `MCP: List Servers` から起動・状態を確認できます。
設定形式は[公式リファレンス](https://code.visualstudio.com/docs/agents/reference/mcp-configuration)にあります。
これは VS Code のチャット向けの例です。ほかのエージェント拡張を使う場合は、その拡張が読む設定も確認します。

MCP の登録では、編集中のソースへの波線表示は追加されません。
LSP のエディタ診断には[クライアント拡張](https://code.visualstudio.com/api/language-extensions/language-server-extension-guide)が必要です。
このリポジトリは現在、VS Code 用の LSP クライアント拡張を同梱していません。
サーバ自体の仕様と既存のクライアント設定例は [LSP の文書](lsp.md)を参照してください。

## 3. エージェントへの指示例

対象プロジェクトの指示ファイルや、タスク開始時のプロンプトに追加します。

```text
Java の名前を付ける前に、ubiqlint.yaml と対象ソースに最も近い terms.yaml を読み、
適用されるルール・用語・振る舞いを確認してください。
MCP の get_context が利用できる場合は、そのツールを使ってください。

コードを変更したら、ubiqlint.yaml のあるディレクトリで ubiqlint check を実行してください。
MCP の check_files を使う場合も、検査したパスと結果を確認してください。
検査できなかった場合は、未実行の理由を報告してください。

辞書を変更するときは ubiqlint term / ubiqlint behavior、または対応する MCP ツールを使ってください。
違反を消すことだけを目的に辞書へ語を追加せず、コードの名前を定義済みの語に合わせてください。
```

## 4. 各クラウド環境での確認

最初は [README の最小例](../README.md#最初の検査)を使い、アプリごとに確認します。

1. 新しいタスクから `ubiqlint --version` を実行できる。
2. 正常な `Order#place()` を CLI で検査すると終了コード 0 になる。
3. サンプルの `place` を `update` に変えると、終了コード 1 と診断がエージェントに届く。
4. `place` に戻すと違反が消える。
5. MCP を追加した場合は、`get_context` と `check_files` を実際に呼び、同じ辞書と診断を取得できる。

確認用サンプル以外のコードや辞書は変更しません。アプリ名だけでなく、
クラウド環境・クライアントの版・CLI/MCP/LSP のどの経路を確認したかも記録してください。
設定ファイルを読み込めたことと、実際にツールを呼び出せたことは別の確認です。
