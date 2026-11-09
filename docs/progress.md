# ubiqlint 進捗と引き継ぎ

最終更新: 2026-11-09（M3 完了時点）

この文書は、ubiqlint の開発を引き継ぐ人やエージェントに向けて、マイルストーンごとの内容と現在の進捗をまとめたものです。背景と設計原則は[構想文書](concept.md)に、仕様と設計判断は[実装計画](plan.md)にあります。ここでは「何ができていて、次に何をするか」に絞って書きます。

## 1. 現在の状況

- M0〜M3 が完了し、すべて main にマージ済みです（PR #1〜#4）
- 次に取りかかるのは M4（パターン照合エンジンと `ubiqlint check`）です
- 辞書・設定ファイル・識別子の分割・Java のシンボル抽出は、それぞれ単体で動きます。ただし、これらを組み合わせてコードの違反を検出する処理はまだありません。CLI にあるのは `ubiqlint validate` だけです

## 2. マイルストーン一覧

| # | 内容 | ゴール | 状態 | PR |
|---|---|---|---|---|
| M0 | Cargo ワークスペースのひな形、CI | `cargo test` が通る | 完了 | #1 |
| M1 | 辞書（`terms.yaml`）と設定ファイル（`ubiqlint.yaml`）の読み込み・検証 | `ubiqlint validate` で、不正な定義を行番号付きで報告できる | 完了 | #2 |
| M2 | 識別子の分割器 | `HTTPOrderClient` などを決定的に分割できる | 完了 | #3 |
| M3 | Java アダプタ | Java ファイルからシンボルを抽出できる | 完了 | #4 |
| M4 | パターン照合エンジンと `ubiqlint check` | 違反を定義済みの候補付きで出力できる | 未着手 | |
| M5 | 辞書の解決規則 | ディレクトリごとに置いた辞書を解決できる | 未着手 | |
| M6 | Java のインポート境界ルール | 外部型とドメイン用語の衝突を検出できる | 未着手 | |
| M7 | 辞書を編集するコマンド | エージェントが辞書ファイルを直接編集せず、コマンドで用語や振る舞いを追加・編集・削除できる | 未着手 | |
| M8 | MCP サーバと Claude Code hooks との連携 | エージェントが辞書を取得し、違反を受け取れる | 未着手 | |

## 3. 次のマイルストーン: M4

ゴールは「違反を定義済みの候補付きで出力できる」ことです。ここで初めて、辞書・設定・識別子の分割・シンボル抽出がつながります。

### 3.1 作るもの

1. **パターン照合エンジン**（コア）
   - 識別子を `identifier::split` で単語列にし、ルールのパターンのどれかに一致するかを調べる
   - パターンの要素は長さが一定でない（`{noun}` が `line item` の 2 語に一致するなど）。そのため、照合には後戻りが必要になる
   - 要素ごとの意味は、計画の 2.1 節のとおり
     - リテラル: その単語列そのもの
     - `{noun}` / `{noun:plural}`: いずれかの用語の単数形 / 複数形
     - `{verb:<form>}`: 結びつく名詞の振る舞いの語形
     - `{<語彙名>}`: 語彙のいずれかの単語列
     - `{any}`: 任意の単語列
2. **`{verb}` と名詞の結びつけ**
   - まず同じ識別子の中で一致した `{noun}` に結びつける
   - 見つからなければ、`Symbols::ancestors` で最も内側の外側の型（`SymbolKind::Type`）をたどり、その名前の用語に結びつける
3. **違反メッセージ**
   - 最も近いパターンとの差分から組み立て、定義済みの振る舞いを候補として示す（計画の 2.2 節）
4. **`ubiqlint check`**（CLI）
   - `ubiqlint.yaml` を読み込み、ルールの `include` を設定ファイルのあるディレクトリからの相対パスとして照合する（`Rule::includes`）
   - 対象のファイルを集め、拡張子でアダプタを選ぶ（`LanguageAdapter::extensions`。今は `JavaAdapter` だけ）
   - シンボルの種類をルールの `target` に対応づける。`constructor` と名前のない型は検査しない
   - 違反と構文エラーを `パス:行:列: error: ...` の形で出し、違反があれば終了コード 1 にする
5. **サンプルプロジェクト**
   - M4 を一通り作ったら、小さな Java のサンプル（例: `examples/order-service`）にかけて、スキーマと設定ファイルの設計が実際のコードに合うかを確かめる（計画の 7 章）

### 3.2 着手前にユーザーと決めること

次の点は計画にまだ書かれていません。実装を始める前に、オーナーに方針を確認してください。

- **M4 で使う辞書**: 辞書の解決規則は M5 で決める。M4 では暫定的に、設定ファイルと同じディレクトリの `terms.yaml` を 1 つだけ使うか、`--dictionary` で指定させるか
- **`{any}` が空の単語列に一致するか**: たとえば `get{any}` が `get` だけの識別子に一致するか
- **同じ識別子に複数のルールが当たったとき**: すべてのルールを満たす必要があるのか、どれか 1 つでよいのか
- **どのファイルを検査するか**: 設定ファイルのディレクトリ以下をすべて走査するのか、引数で渡したファイルだけなのか。`.gitignore` を尊重するか
- **未決事項のうち M4 で避けて通れないもの**（計画の 8 章）
  - 1 つの識別子に名詞が複数含まれるときの `{verb}` の結びつけ方
  - 最も内側の型が名前なしや名詞を含まない場合に、さらに外側の型をたどるか

## 4. M5 以降の概要

- **M5 辞書の解決規則**: ディレクトリごとに置いた `terms.yaml` を、検査するファイルに対して解決する。最も近い辞書だけを使うのか、親とマージするのか、共有カーネルをどう書くかは未決（計画の 8 章）
- **M6 Java のインポート境界ルール**
  - ドメイン層で、ドメイン用語と同じ名前の外部型の `import` と、外部パッケージのワイルドカードインポートを禁止する（計画の 5 章）
  - Java アダプタは、まだ `import` 文を抽出していない。設定ファイルでの書き方も未決
- **M7 辞書を編集するコマンド**: 用語や振る舞いを追加・編集・削除するコマンドを作る。エージェントが辞書ファイルを直接書き換えるのではなく、コマンドを経由して安全に操作できるようにするため。書き込む前に検証し、壊れた辞書や重複した用語ができないようにする。コマンドの体系や、書き換えたときに YAML のコメントや書式を保つかどうかは未決（計画の 8 章）
- **M8 MCP サーバと hooks**: 「ファイルに適用される辞書を返す」と「コード片を検査して違反を候補付きで返す」の 2 つが中心機能。Claude Code の hooks で、編集のたびに検査するループを作る

## 5. コードの構成

| 場所 | 役割 | 主な型・関数 |
|---|---|---|
| `crates/ubiqlint-core/src/config.rs` | 設定ファイルの読み込みと検証 | `Config`、`Rule`（`includes`）、`Target`、`Vocabulary` |
| `crates/ubiqlint-core/src/dictionary.rs` | 辞書の読み込みと検証 | `Dictionary`、`Term`、`NounForms`、`Behavior`、`VerbForms` |
| `crates/ubiqlint-core/src/pattern.rs` | パターンの解析 | `Pattern`、`Element`、`NounForm`、`VerbForm` |
| `crates/ubiqlint-core/src/identifier.rs` | 識別子の分割 | `split` |
| `crates/ubiqlint-core/src/words.rs` | 辞書や設定に書く単語列 | `Words` |
| `crates/ubiqlint-core/src/symbol.rs` | アダプタが返すシンボル | `Symbols`、`Symbol`、`SymbolKind`、`LanguageAdapter` |
| `crates/ubiqlint-core/src/location.rs` | ソース中の位置 | `Location`、`Span`、`LineIndex` |
| `crates/ubiqlint-core/src/diagnostic.rs` | 問題の報告 | `Diagnostic` |
| `crates/ubiqlint-core/src/yaml.rs` | YAML 読み込みの共通処理（非公開） | `Diagnostics`、`Fields` |
| `crates/ubiqlint-java/src/lib.rs` | Java のシンボル抽出 | `JavaAdapter`、`extract` |
| `crates/ubiqlint-java/src/lexical.rs` | Java の字句変換（非公開） | `Translated` |
| `crates/ubiqlint-cli/src/main.rs` | CLI | `validate` サブコマンド |
| `vendor/tree-sitter-java` | 修正した Java の文法（生成物を含む） | `LANGUAGE` |

テストは、コアの単体テスト 55 件、字句変換の単体テスト 8 件、Java アダプタの統合テスト 29 件（`crates/ubiqlint-java/tests/extract.rs`）、CLI の統合テスト 7 件（`crates/ubiqlint-cli/tests/cli.rs`）、doctest 2 件です。
