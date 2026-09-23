# TagEditor

画像タグ付け（アノテーション）用のデスクトップアプリ。ONNX 推論でタグを自動付与し、
タグファイル（`.txt` キャプション）の編集・一括操作・仕訳・改名を 1 つの GUI で行う。
学習データセットの前処理を想定している。

## 主な機能

- **タグの自動付与**: WD14 / ML-Danbooru 系モデルで画像を推論し、確信度付きでタグを付ける。単一画像・バッチ推論に対応し、進捗表示とキャンセルが可能。
- **タグ編集・一括操作**: フォルダ内の画像をサムネイル一覧で表示し、画像ごとのタグを編集。選択した画像群へタグの追加・削除・重複除去をまとめて実行。
- **推論結果の絞り込み**: 確信度やバッチ内の出現割合、除外・置換ルールで、採用するタグを制御。
- **集計・フィルタ**: フォルダ内のタグ出現数を集計し、含む/除外タグで画像を絞り込む。
- **仕訳・振分**: フォルダ間の move / copy、長辺サイズやタグの有無による振分（画像とタグファイルを対で移動）。
- **改名・整理**: 正規表現での改名、ゼロ埋め連番、孤立キャプション（対応画像のない `.txt`）の削除。
- **モデル管理**: 利用可能なモデルの一覧・取得状態表示・ダウンロード。推論時に未取得なら自動取得する。

## 使い方

1. アプリを起動する。
2. 画像フォルダを開くと、サムネイル一覧が表示される。
3. 画像を選んでタグを推論・編集し、保存する（同名の `.txt` に書き出される）。
4. 必要に応じてバッチ推論・一括操作・仕訳・改名を行う。

ビルド済みバイナリは GitHub Release から入手できる。

- Windows: `tag-editor-windows-x86_64.exe`
- Linux: `tag-editor-linux-x86_64`

ONNX 推論を動かすには ONNX Runtime の共有ライブラリ（`onnxruntime.dll` /
`libonnxruntime.so`）が必要。実行ファイルと同じディレクトリに置くか、環境変数
`ORT_DYLIB_PATH` でパスを指定する。共有ライブラリは
[ONNX Runtime のリリース](https://github.com/microsoft/onnxruntime/releases)から入手する。

## 使用技術

- [Tauri v2](https://tauri.app/) — デスクトップアプリのフレームワーク
- [Rust](https://www.rust-lang.org/) — ネイティブ層・コアロジック
- [ONNX Runtime](https://onnxruntime.ai/)（[`ort`](https://crates.io/crates/ort) crate） — 推論エンジン
- 静的な HTML / CSS / JavaScript フロントエンド（ビルド不要）

## 開発

ソースからビルド・実行する場合:

```bash
# 開発起動
cargo run --bin tag-editor

# リリースビルド
cargo build --release --bin tag-editor
```

ビルド/テスト/リリースの詳細な手順は `.kiro/steering/project-ci-config.md` を参照。

## ライセンス

MIT
