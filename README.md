# TagEditor

画像タグ付け（アノテーション）用のデスクトップアプリ。Tauri v2 + Rust ネイティブ層で構築し、
ONNX 推論（WD14 / ML-Danbooru 系タガー）によるタグ自動付与と、タグファイル（`.txt` キャプション）の
編集・一括操作・仕訳・改名を 1 つの GUI で行う。

学習データセットの前処理（タグ付け・整理・命名）を想定している。

## 主な機能

- **画像一覧・プレビュー・タグ編集**: フォルダを開いてサムネイル一覧を表示し、画像ごとの
  タグファイル（同名 `.txt`）を編集・保存する。
- **ONNX 推論によるタグ付け**: WD14 系・ML-Danbooru 系モデルで画像を推論し、確信度付きの
  タグを得る。単一画像・バッチの両方に対応し、進捗表示とキャンセルが可能。
- **ローカルモデル管理**: 選択可能なモデルをカタログから一覧し、取得状態（取得済み/未取得）を
  表示。ダウンロード（進捗・キャンセル対応）してアプリのモデル配置フォルダへ保存する。
  推論実行時に未取得なら自動ダウンロードしてから推論する（遅延ダウンロード）。
- **Tag_Filter（推論結果の採用制御）**: Keep（無条件採用）/ Exclude（除外・正規表現）/
  Replace（検索・置換）/ Additional（無条件付与）/ Confidence_Threshold（確信度下限）/
  Fraction_Threshold（バッチ内出現割合下限）でタグの採用を制御する。
- **Tag_Overview（バッチ後タグ一覧）**: バッチ推論後に採用/不採用タグを確信度付きで一覧表示。
  検索で絞り込み、選択タグを Keep/Exclude/Replace へ送って再推論できる。
- **一括操作**: 選択した画像群へタグの追加・削除・重複除去。
- **集計/フィルタ**: フォルダ内タグの出現数集計と、含む/除外タグによる画像フィルタ。
- **仕訳/サイズ振分/タグ振分**: move/copy/gather/distribute によるフォルダ仕訳、長辺サイズによる
  振分、判定タグの有無による振分（画像とタグファイルを対で移動）。
- **改名/連番/正規化**: 正規表現による改名、ゼロ埋め連番、`basename.ext.txt` → `basename.txt`
  へのファイル名正規化。
- **孤立キャプション削除**: 対応する画像を持たない `.txt` を検出し、承認のうえ削除。
- **Windows 限定機能**: シンボリックリンク作成、Windows ↔ Linux(WSL) パス変換。

## 構成

```
TagEditor/
├─ Cargo.toml            # Rust ワークスペース（バージョンは [workspace.package]）
├─ frontend/             # 静的フロントエンド（index.html / main.js / styles.css）
├─ src-tauri/            # Rust ネイティブ層（Tauri アプリ本体 + コアロジック）
│  ├─ src/
│  │  ├─ logic/          # I/O 非依存の純粋ロジック（タグ処理・命名・仕分け判定 等）
│  │  ├─ services/       # 副作用を伴うサービス（ファイル I/O・推論・モデル取得）
│  │  ├─ commands/       # Tauri コマンド境界（アダプタ・キャンセル・進捗）
│  │  └─ app.rs          # Tauri ランタイム起動・コマンド登録
│  └─ tests/             # 統合テスト・プロパティテスト（proptest）
├─ tests/gui/            # Playwright による GUI E2E テスト（Tauri API モック経由）
└─ .github/workflows/    # CI（ci.yml）・リリース（release.yml）
```

フロントエンドはビルド不要の静的アセット（`frontend/`）で、Rust 層とは Tauri コマンド境界でのみ通信する。

## 開発環境の準備

- [Rust](https://www.rust-lang.org/tools/install)（stable ツールチェーン）
- Tauri v2 の前提依存
  - **Linux**: `libgtk-3-dev` / `libwebkit2gtk-4.1-dev` / `libayatana-appindicator3-dev` /
    `librsvg2-dev` / `libssl-dev` / `patchelf`
  - **Windows**: Microsoft Visual C++ Build Tools（MSVC）と WebView2 ランタイム
- ONNX 推論を実際に動かす場合は ONNX Runtime 共有ライブラリ（後述「ONNX Runtime について」）
- Playwright テストを動かす場合は Node.js と `pnpm`（または `npm`）

## 開発時の起動

`@tauri-apps/cli` は未導入で、ネイティブアプリのエントリポイント（`src-tauri` の
`tag-editor` バイナリ）を直接起動する。

```bash
# リポジトリルートから
cargo run --bin tag-editor
```

`src-tauri/tauri.conf.json` の `frontendDist` が `../frontend` を指しており、静的アセットが
そのまま読み込まれる。

## ビルド

```bash
# リリースバイナリをビルド
cargo build --release --bin tag-editor
```

生成物: `target/release/tag-editor`（Windows は `tag-editor.exe`）。

現状 `tauri.conf.json` の `bundle.active` は `false` のため、インストーラ（.msi/.deb 等）は
生成せず、素の実行バイナリのみを配布する。

## テスト

```bash
# フォーマットチェック
cd src-tauri && cargo fmt --check

# 静的解析（警告をエラー扱い）
cargo clippy --lib -- -D warnings

# ユニット/統合/プロパティテスト（--release は画像プロパティテストの高速化のため）
cargo test --workspace --release
```

実 `ort::Session` 構築に到達する一部テストは `#[ignore]` 済みで、通常のテスト実行では走らない
（理由は「ONNX Runtime について」を参照）。ローカルで ONNX Runtime を配置した環境で検証する場合は
`cargo test -- --ignored` で個別に実行する。

GUI の動作確認（Playwright、任意）:

```bash
pnpm install
pnpm test:e2e
```

## ONNX Runtime について

推論は [`ort`](https://crates.io/crates/ort) クレート（`load-dynamic` 機能）を使い、ONNX Runtime の
共有ライブラリ（`onnxruntime.dll` / `libonnxruntime.so` / `libonnxruntime.dylib`）を実行時に動的
ロードする。ライブラリの探索順序:

1. 環境変数 `ORT_DYLIB_PATH`（ファイルまたはディレクトリへのパス）
2. 実行ファイルと同じディレクトリ

いずれも見つからない場合は ONNX Runtime 既定のシステム検索にフォールバックする。配布時は
[ONNX Runtime のリリース](https://github.com/microsoft/onnxruntime/releases)から対応する共有
ライブラリを取得し、実行ファイルと同じディレクトリに置く。

CI では ONNX Runtime を配置していない。ONNX Runtime のプロセス終了時解放処理は `ort` クレート内部の
実装詳細で環境（dlopen/dlclose 順序）によっては終了時に異常終了することがあり、CI からは制御できない
ため、実 `ort::Session` 構築に到達するテストは `#[ignore]` で除外している。

## リリース

`.github/workflows/release.yml` が `v*` タグの push を契機に、Windows / Linux 向けのリリース
バイナリをビルドして GitHub Release へ添付する。**master へ push しただけではリリースは作成されない**。

1. 3 ファイルのバージョンを揃える: ルート `Cargo.toml` の `[workspace.package]`、
   `package.json`、`src-tauri/tauri.conf.json`
2. `RELEASE_NOTES.md` を更新（リリース本文にそのまま使われる）
3. master へマージ・push
4. `v{version}` タグを作成して push

```bash
git tag -a v0.2.1 -m "v0.2.1"
git push origin v0.2.1
```

配布物:

- Windows: `tag-editor-windows-x86_64.exe`
- Linux: `tag-editor-linux-x86_64`

## ライセンス

MIT
