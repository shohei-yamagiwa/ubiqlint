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
