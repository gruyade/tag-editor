# TagEditor v0.2.1

## 変更点

### バグ修正

- Linux CI でテストが SIGABRT（signal 6）で異常終了する問題を修正。バックグラウンド推論スレッドをテスト終了時に確実に回収（join）し、デタッチスレッドが共有状態を汚染して並列実行下で二重パニックを起こすのを防止

### 改善

- CI のテストを並列実行（`--test-threads=1` を撤去）かつ `--release` 実行へ変更し、テスト時間を短縮
- Windows / Linux 向けのリリースビルドを GitHub Actions で自動化（`v*` タグの push で両 OS のバイナリをビルドし GitHub Release へ添付）

## ダウンロード

GitHub Release から実行ファイルをダウンロードしてください。

- Windows: `tag-editor-windows-x86_64.exe`
- Linux: `tag-editor-linux-x86_64`
