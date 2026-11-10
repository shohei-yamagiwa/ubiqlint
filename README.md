# ubiqlint

A universal linter for a ubiquitous language

ユビキタス言語の辞書を使って、Java の識別子の命名とインポート境界を検査するツールです。
たとえば `Order` の振る舞いを `place`・`cancel` と定義し、`update` のような辞書にない
振る舞いをメソッド名として使ったときに指摘できます。

開発中です。現在の対応言語は Java です。CLI、設定・辞書の形式、診断内容、MCP ツールは、
0.x の間はマイナーバージョンでも互換性を保たずに変更する場合があります。
当面は個人利用を主目的に、ソースからインストールして使います。
一般向けの Release やビルド済みバイナリの配布は、必要になった時点で検討します。

## できること

- 型・メソッド・フィールド・引数・ローカル変数の名前を、辞書とパターンに照らして検査する
- ソースに最も近い `terms.yaml` を使い、ディレクトリごとに辞書を分ける
- 辞書の用語と衝突する外部型のインポートなど、設定したインポート境界の違反を検出する
- CLI・MCP から新しいコンテキストの空の辞書を作成する
- CLI から辞書の用語・振る舞いを追加・編集・削除する
- MCP から辞書の取得・編集とコードの検査を行う
- 診断専用の LSP から、未保存の Java バッファを検査する

## インストール

ソースからのビルドには、Rust 1.88 以上（最新の stable を推奨）、Cargo、
C11 対応の C コンパイラとリンカが必要です。Git から取得する場合は Git も必要です。
生成済みの Java パーサーを同梱しているため、Node.js や tree-sitter CLI は不要です。
検査の実行に JDK は必要ありません。

```sh
git clone https://github.com/shohei-yamagiwa/ubiqlint.git
cd ubiqlint
cargo install --path crates/ubiqlint-cli --locked
ubiqlint --version
```

Cargo のインストール先（通常は `$HOME/.cargo/bin`）に PATH を通してください。
CLI・MCP・LSP は、すべて同じ `ubiqlint` コマンドに含まれます。

作業用の clone を作らず、Git の最新版から直接インストールすることもできます。

```sh
cargo install --git https://github.com/shohei-yamagiwa/ubiqlint.git --locked ubiqlint-cli
```

バージョンタグの公開後は、`--tag <タグ名>` を追加すると特定の版を指定できます。

## 最初の検査

空のディレクトリに、次の 3 ファイルを作成します。

```text
ubiqlint.yaml
terms.yaml
src/
  Order.java
```

`ubiqlint.yaml` は、対象ファイルと許可する名前のパターンを指定します。

```yaml
rules:
  - id: domain-type
    include: ["src/**/*.java"]
    target: type
    patterns: ["{noun}"]
  - id: domain-method
    include: ["src/**/*.java"]
    target: method
    patterns: ["{verb}"]
```

`terms.yaml` は、用語とその振る舞いを定義します。名詞の複数形と動詞の活用形は、
推測せず明示的に指定します。

```yaml
context: order
terms:
  - name: Order
    ja: 注文
    forms: { singular: order, plural: orders }
    behaviors:
      - verb: place
        forms: { base: place, s: places, past: placed, pp: placed, ing: placing }
```

`src/Order.java` には、辞書にある名前を使います。

```java
class Order {
    void place() {}
}
```

このディレクトリで実行します。正常なら何も出力せず、終了コード 0 で終了します。

```sh
ubiqlint validate ubiqlint.yaml terms.yaml
ubiqlint check
```

`place` を `update` に変えると、`ubiqlint check` は次の診断を出し、終了コード 1 になります。

```text
src/Order.java:2:10: error: `update` does not match pattern `{verb}` of rule `domain-method`: `update` is not a defined behavior of `Order` (defined behaviors: `place`)
```

複数のコンテキストとインポート境界を扱う例は、[order-service サンプル](examples/order-service/README.md)
を参照してください。リポジトリのルートから、次のコマンドで検査できます。

```sh
ubiqlint check --config examples/order-service/ubiqlint.yaml
```

## CLI の使い方

```sh
ubiqlint check                         # 設定配下を検査
ubiqlint check src/Order.java          # 指定したファイルだけ検査
ubiqlint check --config path/to/ubiqlint.yaml
ubiqlint validate ubiqlint.yaml terms.yaml
ubiqlint dictionary init --context order  # 空の辞書を新規作成
ubiqlint term --help                   # 用語の追加・編集・削除
ubiqlint behavior --help               # 振る舞いの追加・編集・削除
ubiqlint --help
```

新しいコンテキストの辞書を作成する場合は、保存先とコンテキスト名を指定します。

```sh
ubiqlint dictionary init --dictionary src/order/terms.yaml --context order
```

保存先の親ディレクトリは事前に用意します。既存のパスは上書きしません。
`--dictionary` の既定値は `terms.yaml` で、`--context` は必須です。
空の辞書でも、その配下では親の辞書が適用されなくなります。
作成後は `ubiqlint term add` で用語を追加できます。

既定の設定はカレントディレクトリの `ubiqlint.yaml` です。検査対象は設定ファイルの
ディレクトリ以下で、各ソースから上にたどって最初に見つかる `terms.yaml` を使います。
親や兄弟の辞書とのマージは行いません。

`check`・`validate` の診断は標準出力に `path:line:column: error: message` の形式で出ます。
警告は標準エラー出力です。検査対象が 0 件の場合は警告を出しますが、それだけでは失敗しません。

| 終了コード | 意味 |
| --- | --- |
| 0 | 正常終了。警告だけの場合も含む |
| 1 | 検査違反、設定・辞書の不正、ファイルの読み書きの失敗など |
| 2 | 必須引数の不足など、CLI の引数の誤り |

## MCP・LSP で使う

Windows の Desktop・Web アプリからクラウド上のコードを扱う場合は、
[Claude Code・Codex・Mistral Vibe・VS Code の導入手順](docs/clients.md)を参照してください。
CLI はコードがある作業環境に導入し、MCP・LSP はその環境の対応状況に合わせて追加します。

- **MCP**: `ubiqlint mcp` を stdio サーバとして登録します。
  辞書の取得・編集とコード検査の使い方は[エージェントとの連携](docs/agents.md)を参照してください。
- **LSP**: `ubiqlint lsp` を追加の stdio LSP サーバとして登録します。
  Claude Code プラグインと Neovim の設定は [LSP の文書](docs/lsp.md)にあります。

LSP は診断専用です。補完・定義ジャンプ・コードアクションは提供しません。
Claude Code 2.1.319 の LSP 管理処理では、同じ拡張子の複数サーバに編集通知が配信されないため、
`jdtls-lsp` と ubiqlint の両方の診断を受け取れません。併用確認の方法と回避策は
[LSP の文書](docs/lsp.md#jdtls-lsp-との併用確認)を参照してください。

## 開発とサポート

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

個人利用を主目的に開発しています。不具合報告や改善提案は GitHub Issues で受け付けますが、
対応やリリース時期はベストエフォートです。
現在の機能と制約は [CHANGELOG](CHANGELOG.md)、
個人利用に向けた確認結果と将来の配布手順は[配布準備](docs/releasing.md)にまとめています。

## ライセンス

ubiqlint は [MIT ライセンス](LICENSE)で配布します。
同梱する改変版 tree-sitter-java のライセンスと改変内容は、
[vendor 内の文書](vendor/tree-sitter-java/README.md)を参照してください。
