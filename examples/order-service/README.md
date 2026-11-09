# order-service サンプル

ubiqlint の動作確認用の小さな Java プロジェクトです。スキーマ（`terms.yaml`）と設定（`ubiqlint.yaml`）が実際のコードに合うかを確かめるために使います。

辞書はパッケージのディレクトリごとに置きます。プロジェクトのルートには辞書を置いていません。

```text
ubiqlint.yaml
src/main/java/com/example/
  order/
    terms.yaml       # Order、LineItem とその振る舞い
    domain/          # Order、OrderPlaced、OrderRepository、LineItem
    dto/             # OrderDto
  billing/
    terms.yaml       # Invoice と issue
    domain/          # Invoice
```

`order/` 以下のソースには注文の辞書、`billing/` 以下には請求の辞書だけが適用されます。親や兄弟の辞書とはマージしません。たとえば `billing/domain/Order.java` を追加すると、注文の辞書に `Order` があっても違反になります。

リポジトリのルートから次のように実行すると、違反がなければ終了コード 0 で何も出力しません。

```
cargo run -p ubiqlint-cli -- check --config examples/order-service/ubiqlint.yaml
```

`ubiqlint.yaml` の `import_rules` は、注文と請求それぞれのドメイン層のインポート境界を検査します。注文側は `com.example.order` 以下を内部とし、それ以外から `Order` や `LineItem` をインポートすると違反になります。請求側は `com.example.billing` 以下を内部とし、最も近い請求の辞書にある `Invoice` との衝突を検査します。外部のワイルドカードインポート（`import java.util.*;` など）も違反です。`Order.java` にある `java.util.List` のような、辞書の用語と衝突しない明示的なインポートは許可されます。

同名の外部型を使う場合は、インポートを追加せず `com.vendor.Order` のように完全修飾名で参照します。DTO はこのインポートルールの `include` に含まれないため、対象外です。

## 辞書を編集する

辞書の用語や振る舞いは、ファイルを直接書き換えずにコマンドで追加・編集・削除します。たとえば、注文に「発送する（ship）」という振る舞いを追加するには、リポジトリのルートから次のように実行します。

```
cargo run -p ubiqlint-cli -- behavior add Order ship --s ships --past shipped --pp shipped --ing shipping \
  --dictionary examples/order-service/src/main/java/com/example/order/terms.yaml
```

語形は推測しないので、すべて指定します。コマンドは書き込む前に辞書を検証し、変更した行以外のコメントや書式はそのまま残します。追加すると `Order#ship()` や `OrderShipped` が命名規則を満たすようになります。サンプルを元に戻すには `git checkout examples/order-service` を実行します。
