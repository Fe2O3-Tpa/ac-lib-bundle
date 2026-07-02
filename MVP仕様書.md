# MVP仕様書 — Rust提出用ライブラリ展開ツール

## 目的

`src/bin/*.rs`を入力とし、`src/lib.rs`以下の自作ライブラリを展開して、提出可能な単一のRustソースコードを生成する。

対象はAtCoderを想定する。

---

# 対象範囲

## 入力

```text
src/
├── lib.rs
├── graph.rs
├── math/
│   ├── mod.rs
│   └── gcd.rs
└── bin/
    └── abc100_a.rs
```

## 出力

出力先は次の優先順位で決定する。

1. `-o output.rs`
2. `--stdout`
3. 省略時は標準出力

`-o` と `--stdout` が同時指定された場合はエラーとする。

---

# 対応機能

## 1. mod展開

対応

```rust
mod graph;
```

```rust
pub mod graph;
```

探索順

```
graph.rs
graph/mod.rs
```

---

## 2. lib.rsの展開

対応

```rust
use mylib::graph::Graph;
```

↓

`src/lib.rs`

↓

`graph.rs`

まで辿る。

---

## 3. ネストしたmod

対応

```rust
mod math;
```

math/mod.rs

↓

```rust
mod gcd;
```

↓

math/gcd.rs

---

## 4. pub

保持する。

```rust
pub struct Graph;
```

↓

そのまま出力

---

## 5. use の変換方針

`use mylib::...` のような自作ライブラリ参照は、展開後の単一ファイルで解決できる形に書き換える。
標準ライブラリや外部クレートへの `use` は変更しない。

`crate::...`、`super::...`、`self::...` は展開対象外とする。

---

## 6. extern crate

そのまま

---

## 7. マクロ

対応

```rust
macro_rules! chmin
```

```rust
macro_rules! input
```

そのままコピー

---

## 8. const

対応

---

## 9. static

対応

---

## 10. trait

対応

---

## 11. impl

対応

---

## 12. enum

対応

---

## 13. struct

対応

---

## 14. type alias

対応

---

## 15. fn

対応

---

# 非対応

## proc_macro

未対応

---

## build.rs

未対応

---

## feature

未対応

---

## workspace

未対応

---

## include!

未対応

---

## include_str!

未対応

---

## include_bytes!

未対応

---

## #[path]

未対応

---

## mod inside function

未対応

---

## pub(in ...)

未対応

---

## unsafe extern

未対応

---

## generic const expr

考慮しない

---

# 展開アルゴリズム

```
入力(bin/*.rs)

↓

構文解析(syn)

↓

crate名取得

↓

use crate_name::...

↓

lib.rs

↓

mod探索

↓

DFS

↓

AST収集

↓

mod宣言削除

↓

use crate_name::...削除

↓

出力
```

## crate名の決定

crate名は `Cargo.toml` の `[package] name` を優先して取得する。
`Cargo.toml` が取得できない場合はエラーとする。
```
error:
cannot resolve crate
```

## mod探索の基準

各 `mod foo;` は、現在のモジュールファイルと同じ階層から次の順で探索する。

1. `foo.rs`
2. `foo/mod.rs`

探索元は展開中のモジュールの実ファイル位置を基準とする。

## cfg系の扱い

`#[cfg(...)]`、`#[cfg_attr(...)]` は解析対象から除外する。
ただし、これらにより展開に必要な `mod` や `use` が隠れる場合は未対応としてエラーとする。

## 保持する情報

展開時は、元ファイル中の以下を可能な限り保持する。

- `pub` を含む可視性
- 属性
- コメント
- `use`
- `extern crate`
- `macro_rules!`

ただし、モジュール展開のために不要な `mod` 宣言は削除する。

## 重複展開の判定

同一モジュールの重複判定は、正規化後の絶対パスで行う。
相対パス表記の違いによる重複は同一モジュールとして扱う。

## 循環参照

展開中のモジュールを再度たどった場合は循環参照とみなし、エラーとする。
```
error:
cyclic module detected
```

---

# 重複展開

同一モジュールは一度だけ展開する。

```
visited: HashSet<PathBuf>
```

管理する。

---

# 出力順

DFS順。

例

```
lib.rs

↓

graph.rs

↓

graph/search.rs

↓

math.rs

↓

main.rs
```

---

# エラー

対象ファイルが存在しない

```
error:
module "graph" not found
```

循環参照

```
error:
cyclic module detected
```

lib.rsが存在しない

```
error:
lib.rs not found
```

crate名が一致しない

```
error:
cannot resolve crate
```

## エラー出力

エラーメッセージは以下の形式に従う。

- `module "..." not found`
- `cyclic module detected`
- `lib.rs not found`
- `cannot resolve crate`

文言は原則としてこの形式に合わせる。

---

# CLI

```
bundle <input.rs>
```

例

```
bundle src/bin/abc350_f.rs
```

---

# オプション

```
-o output.rs
```

```
--stdout
```

---

# MVPで保証すること

* `src/lib.rs`をエントリとする通常のモジュール構成を展開できる
* `mod foo;`・`pub mod foo;`・`foo.rs`・`foo/mod.rs`に対応する
* 同一モジュールは一度だけ展開する
* 自作ライブラリのみ展開し、標準ライブラリや外部クレート（`std`、`proconio`、`itertools`、ACLなど）は展開しない

## コンパイル保証の範囲

生成結果が `rustc` でコンパイル可能であることを目標とする。
ただし、入力プロジェクト自体がコンパイル可能であり、かつ未対応機能を含まない場合に限る。
