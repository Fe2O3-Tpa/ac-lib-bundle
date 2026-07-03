# ac-lib-bundle

Rust の提出用コードを 1 ファイルに束ねる CLI ツールです。

## 使い方

### `ac-lib-bundle {path_to_target}`

`{path_to_target}`のバンドル結果を標準出力に出力します。
`--stdout`をつけたときと同じです。

### `ac-lib-bundle {path_to_target} -o output.rs`

`{path_to_output}`に`{path_to_target}`のバンドル結果を出力します。
上書きされるので注意してください。

### `ac-lib-bundle {path_to_target} --stdout`

`{path_to_target}`のバンドル結果を標準出力に出力します。

## 何をするか

- 入力の `*.rs` を `syn` で構文解析する
- `Cargo.toml` をたどって、入力ファイルが属する workspace / package を特定する
- `use my_lib::...` のような参照から、`Cargo.toml` の path dependency を解決する
- その dependency が `my_lib` ではなく任意名でも、その名前を使って探索する
- 自作 crate の `src/lib.rs` と必要な `mod foo;` だけを展開して、単一ファイルにまとめる
- 標準ライブラリや外部 crate は展開しない
- `mod foo;` は、そのファイル内で `foo::` が参照されている場合のみ展開する

## 探索アルゴリズム

1. 入力ファイルから上方向へ `Cargo.toml` を探す
2. その `Cargo.toml` を workspace / package の起点として読む
3. `[dependencies]` などの path dependency を収集する
4. ソース中の `alias::` を見つけたら、その alias に対応する local crate を展開する
5. 親から必要とされた `mod foo;` だけを展開する
6. `mod foo;` は、そのファイル内で `foo::` が参照されている場合だけ `foo.rs` / `foo/mod.rs` を探して再帰展開する
7. 展開済み crate は `visited` で 1 回だけにする

## 例

`Cargo.toml` で次のように定義されていても、

```toml
[dependencies]
my_lib = { path = "libs" }
```

ソース側で

```rust
use my_lib::common::yn;
```

と書かれていれば、`my_lib` を local crate として解決して展開します。

## 備考

- `cfg` 系は未対応です
- `include!` 系は未対応です
- workspace 配下の path dependency を前提にしています
- `mod` は「そのファイル内で実際に参照されているもの」または「親から要求されたもの」だけを再帰展開します
