# TagEditor v0.2.0

## 変更点

### 新機能

- ローカルモデル管理: 静的カタログからのモデル取得・保存・読込をローカル配置フォルダ基準に再設計。存在判定バッジ、ダウンロード/キャンセル、進捗表示に対応（Model_Management_Tab）
- タグフィルタ: Keep / Exclude / Replace / Additional と 2 種の閾値（Confidence / Fraction）を持つ Tag_Filter を導入。Replace → Keep → Exclude/Threshold → Additional の順で採用判定
- バッチ集計: 推論結果を採用/不採用の 2 区分で表示する Tag_Overview を追加。検索、Keep/Exclude への送出、再推論に対応
- 遅延ダウンロード: 未取得モデルは推論開始時に自動ダウンロード（30 秒 × 3 回再試行・原子的保存・ロールバック）してから推論
- プレビュー転送の高速化: サムネイル/プレビューを Asset Protocol 経由のファイルパスで転送し、キャッシュで再エンコードを削減
- GUI テスト環境: Playwright を導入し、Tauri API モック経由で frontend/main.js のロジックを検証

### バグ修正

- 推論・モデルダウンロード・プレビュー転送のアプリ層結線欠落（4 系統）を修正。ModelSessionState によるセッション保持、start_inference / spawn_model_download の起動結線を追加
- Tag_Overview の image_count で、他画像の Discarded 出現を Adopted 統計へ誤算入する問題を修正

### 改善

- バッチ推論の対象選択を画面上部の選択モードに統一（対象ラジオボタンを廃止）
- Additional_Tags を採用タグの先頭へ付与し、重複を正規化キーで除去（先勝ち）

### 破壊的変更

- データモデルを再設計: ModelLocation を廃し ModelSource を導入、ModelFamily::Local を廃止、InferResult を overview 付きの InferBatchResult へ拡張

## ダウンロード

Windows 用インストーラー (.exe) をダウンロードして実行してください。
