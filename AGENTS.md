# AGENTS.md
Rust製のAtCoderユーザー向けバンドルツール「ac-lib-bundle」の開発で使用されるエージェントに関するドキュメントです。
## プロジェクト概要
ac-lib-bundleは、AtCoderのユーザーが競技プログラミングのために必要なローカルに存在するライブラリを簡単に利用できるようにするためのバンドルツールです。
`ac-lib-bundle ./src/main.rs`のようにファイルを指定して実行すると、ローカルに存在してかつ実際にファイルで使われているクレートをバンドルします。
### 例(ver0.3.1による)
```bash
ac-lib-bundle ./src/main.rs
```

```rust
use my_lib::graph::Graph;
fn main() {
    let mut g = Graph::new(5);
    g.add_edge(0, 1);
    println!("{:?}", g);
}
```
↓
```rust
mod graph {
    #[derive(Debug)]
    pub struct Graph {
        ...
    }

    impl Graph {
        ...
    }
}
use crate::graph::Graph;
fn main() {
    let mut g = Graph::new(5);
    g.add_edge(0, 1);
    println!("{:?}", g);
}
```
### プロジェクトの環境
- rustc 1.96.0
#### 依存関係等
- pathdiff
- proc-macro2
- serde
- syn
- toml
## 開発フロー
AIがやるべき部分は(AI)と項目の最初に書きます。
1. pullする
2. 作業ブランチを切る
3. (AI)コードを書く
4. (AI)コードをコミットする
    - ここで、人間によるレビューが入る
    - 不可であればRevertが入る
5. (AI)3と4を繰り返す
6. プルリクエストを作成する
7. レビューを受ける
8. マージする

5から6は人間が行う。
## エージェントの役割
ac-lib-bundleの開発において、エージェントはコード作成、コードのレビュー、定型作業(章「開発フロー」の1など)を担当する。
