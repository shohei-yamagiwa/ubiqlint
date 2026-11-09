# コーディングエージェントとの連携

この文書は、コーディングエージェントに ubiqlint を使わせるための手順をまとめたものです。設計の背景と仕様は[実装計画](plan.md)の 6.2 節（MCP サーバ）と 6.4 節（hooks）にあります。

エージェントには、MCP サーバ `ubiqlint mcp` を通じて次のことをさせます。

- 名前を付ける前に、そのファイルに適用される辞書とルールを調べる
- コードを書いたら、CI と同じ検査をかけて違反を直す
- ユビキタス言語に語を加えるときは、辞書ファイルを直接書き換えず、専用のツールで編集する

違反を消すためだけに辞書を変えていないかは、レビューするエージェントが確かめます（5 節）。

編集のたびに違反を自動で受け取るには、診断専用の `ubiqlint lsp` を使います。[LSP の設定手順](lsp.md)に Claude Code のプラグインと Neovim の設定例があります。Claude Code では jdtls と同じ `.java` に通知を配信できない制限があるため、Java の LSP を選んで使うか、jdtls と ubiqlint の MCP・hooks を組み合わせます。

## 1. MCP サーバを登録する

`ubiqlint mcp` は、標準入出力で MCP を話すサーバです。1 つのサーバが 1 つの設定ファイル（`ubiqlint.yaml`）のプロジェクトを受け持ちます。

```sh
ubiqlint mcp --config path/to/ubiqlint.yaml
```

- `--config` を省略すると、サーバを起動したディレクトリの `ubiqlint.yaml` を使います。クライアントがプロジェクトのルート以外でサーバを起動する場合や、設定ファイルがルートにない場合は、`--config` で指定してください
- サーバは起動すると設定ファイルのディレクトリに移り、ツールが受け取るパスも返すパスも、そのディレクトリからの相対パスで扱います。絶対パスも受け付けます
- 設定ファイルが見つからない、または不正でもサーバは起動し、各ツールがその問題を返します
- MCP の 2026-07-28 版と、`initialize` で始まる 2025-11-25 以前の版の両方に対応しています

Claude Code では、次のように登録します。

```sh
claude mcp add ubiqlint -- ubiqlint mcp
```

チームで共有する場合は、リポジトリのルートに `.mcp.json` を置きます。

```json
{
  "mcpServers": {
    "ubiqlint": {
      "command": "ubiqlint",
      "args": ["mcp"]
    }
  }
}
```

ほかのクライアントでも、stdio のサーバとして `ubiqlint mcp --config <設定ファイル>` を起動するよう設定すれば使えます。

## 2. ツール

| ツール | 使いどころ |
|---|---|
| `get_context` | 新しい型やメソッドに名前を付ける前。ファイルのパス（まだ存在しなくてよい）を渡すと、適用される命名ルール・インポートルール・語彙と、辞書（最も近い `terms.yaml`）の内容をそのまま返す |
| `check_code` | ファイルに書き込む前。下書きの全文を、そのパスに置いたものとして検査する |
| `check_files` | ファイルを編集した後。指定したファイルやディレクトリ、または全体を、CI の `ubiqlint check` と同じように検査する。既定では違反を 100 件まで返す |
| `add_term`・`edit_term`・`remove_term` | 用語を追加・編集・削除する。`ubiqlint term` と同じ |
| `add_behavior`・`edit_behavior`・`remove_behavior` | 振る舞いを追加・編集・削除する。`ubiqlint behavior` と同じ |

検査の結果は、CLI と同じ `path:line:column: error: message` の行と、最後の件数の行で返ります。

```text
src/main/java/com/example/order/domain/Order.java:7:17: error: `updateOrder` does not match pattern `{verb}{noun}` of rule `domain-method`: `update` is not a defined behavior of `Order` (defined behaviors: `place`, `cancel`)
1 problem in `src/main/java/com/example/order/domain/Order.java`.
```

`exclude` に一致するファイルや、どのルールも適用されないファイルは「検査しなかった」と理由付きで返ります。違反がないこととは区別され、`check_files` の最後の件数にも含まれません。ルートの辞書が不正な場合は、CI と同じく、どのファイルを検査するときにも問題として返ります。パスが範囲外の場合、設定ファイルや辞書が不正な場合、編集が拒否された場合は、ツールの結果がエラー（`isError`）になり、理由がすべて本文に書かれます。

辞書の編集ツールは、語形を推測しません。複数形や動詞の活用形はすべて引数で渡します。編集した辞書は書き込む前に検証され、コメントや書式は変更した行以外そのまま残ります（[実装計画](plan.md)の 3.2 節）。

## 3. Claude Code の権限設定

実装からレビューまでをエージェントに任せる運用では、ubiqlint のツールを一律に許可し、辞書ファイルの直接の編集だけを禁止します。`.claude/settings.json` に次のように書きます。

```json
{
  "permissions": {
    "allow": ["mcp__ubiqlint__*"],
    "deny": ["Edit(terms.yaml)"]
  }
}
```

- `mcp__ubiqlint__*` は、`ubiqlint` という名前で登録したサーバのすべてのツールに一致します
- `Edit` のルールは、ファイルを編集する組み込みのツールすべてに適用されます。ファイル名だけのパターンは gitignore と同じく、どの深さのディレクトリにも一致します
- シェルのコマンド（`sed -i` など）による書き換えは、このルールでは止められません。辞書の差分は、レビュー（5 節）で必ず確かめてください

## 4. hooks から検査する（任意）

ubiqlint は、特定のエージェントの hooks に対応するコードを持ちません（[実装計画](plan.md)の 6.4 節）。自動診断には LSP を使えますが、LSP を起動できない環境や jdtls を優先する Claude Code では、ファイルを指定する `ubiqlint check` を hooks から呼べます。

Claude Code では、ファイルを編集した後に呼ばれる `PostToolUse` に登録します。終了コード 2 で終わると、stderr の内容が Claude に渡ります。

```json
{
  "hooks": {
    "PostToolUse": [
      {
        "matcher": "Edit|Write",
        "hooks": [
          { "type": "command", "command": "\"$CLAUDE_PROJECT_DIR\"/scripts/ubiqlint-hook.sh" }
        ]
      }
    ]
  }
}
```

```sh
#!/bin/sh
# Checks the file Claude Code has just edited. On violations, exits with 2
# so that Claude sees them on stderr.
file=$(jq -r '.tool_input.file_path // empty')
case "$file" in
  "$CLAUDE_PROJECT_DIR"/*) ;;
  *) exit 0 ;;
esac
cd "$CLAUDE_PROJECT_DIR" || exit 0
if ! problems=$(ubiqlint check "$file" 2>/dev/null); then
  printf '%s\n' "$problems" >&2
  exit 2
fi
```

ほかのエージェントでも、hooks からこのスクリプトと同じことをすれば使えます。ただし、編集したファイルのパスの受け取り方と、結果をモデルに返す方法はエージェントごとに違い、返せないものもあります。それぞれのエージェントの文書を確かめてください。

pre-commit などから、変更したファイルだけを検査することもできます。

```sh
files=$(git diff --cached --name-only --diff-filter=ACMR -- '*.java')
[ -z "$files" ] || ubiqlint check $files
```

ファイルを渡さないと全体を検査するので、変更したファイルがないときは呼ばないようにします。

## 5. 辞書の変更をレビューする

ubiqlint のツールは、辞書が壊れないことは保証しますが、変更がユビキタス言語として正しいかは判断しません。辞書の変更を含む差分をレビューするエージェントには、次の観点を確かめさせます。レビュー用の指示（`AGENTS.md` や `CLAUDE.md`、レビューのプロンプトなど）に含めておくとよいでしょう。

- **違反を消すためだけの変更ではないか**: 追加した用語や振る舞いが、その差分で増えたコードの名前にしか使われていない場合は疑う。違反は、定義済みの用語と振る舞いを使ってコードの名前を直すのが先
- **汎用的な動詞ではないか**: `update`、`set`、`create`、`delete`、`process`、`handle`、`manage` のような CRUD や汎用の動詞を、ドメインの振る舞いとして足していないか。ドメインでの意味（`cancel`、`ship` など）で言い換えられないか
- **同義語ではないか**: 既にある用語の言い換え（`Order` に対する `Purchase` など）を、別の用語として足していないか
- **ドメインの言葉として説明できるか**: 用語に `description` があり、要件や仕様、ドメインの専門家の言葉に照らして説明できるか
- **置き場所が正しいか**: その語は、変更した `terms.yaml` のコンテキスト（境界づけられたコンテキスト）に属するか。別のコンテキストの語を持ち込んでいないか
- **語形が正しいか**: 不規則変化や、`canceled` と `cancelled` のような綴りの揺れが、既存の振る舞いと食い違っていないか
- **削除や名前の変更にコードが追随しているか**: 用語や振る舞いを消したり名前を変えたりした場合に、`ubiqlint check` が通るか

これらに当てはまる変更は差し戻し、コードの名前を定義済みの語に合わせるよう求めます。新しい語が本当に必要なら、その理由を説明させます。
