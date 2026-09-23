# TagEditor v0.2.4

## 変更点

### ドキュメント

- README を利用者向けに再作成。何ができるか・使い方・使用技術・ライセンスを中心に整理し、開発者向けの詳細手順は分離
- プロジェクトの CI・リリース設定ステアリングに、README 作成方針とドキュメント運用の記載を追加

### 改善

- バージョン表記を 3 ファイル（`Cargo.toml` / `package.json` / `tauri.conf.json`）で 0.2.4 に統一。これまで内部バージョンがタグ（v0.2.3）に追従しておらず 0.2.1 のままだった齟齬を解消し、リリースページ・`CARGO_PKG_VERSION` に正しいバージョンが反映されるようにした

## ダウンロード

GitHub Release から実行ファイルをダウンロードしてください。

- Windows: `tag-editor-windows-x86_64.exe`
- Linux: `tag-editor-linux-x86_64`
