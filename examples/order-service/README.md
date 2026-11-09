# order-service サンプル

ubiqlint の動作確認用の小さな Java プロジェクトです。スキーマ（`terms.yaml`）と設定（`ubiqlint.yaml`）が実際のコードに合うかを確かめるために使います。

リポジトリのルートから次のように実行すると、違反がなければ終了コード 0 で何も出力しません。

```
cargo run -p ubiqlint-cli -- check --config examples/order-service/ubiqlint.yaml
```
