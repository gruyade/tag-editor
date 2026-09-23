---
inclusion: manual
---

# プロジェクト固有: CI・リリース設定

このファイルはマージワークフロー（merge-workflow.md）から参照される、
プロジェクト固有のCI設定・バージョニング・リリース情報。

---

## CIチェックコマンド

TagEditor には CI 用のフロントエンドチェック（lint/type-check/test）は
存在しない。フロントエンドは静的な `frontend/main.js` であり、
`package.json` には Playwright による GUI E2E テスト（`test:e2e`）のみが
定義されている。

### バックエンド（Rust / Tauri）

```bash
# フォーマットチェック
cd src-tauri && cargo fmt --check

# 静的解析（警告をエラーとして扱う）
cargo clippy --lib -- -D warnings

# テスト実行（--release: debugビルドの画像プロパティテストが極端に遅いため。
# 並列実行で問題ない。実 ort::Session 構築に到達するテストは #[ignore] 済み）
cargo test --workspace --release
```

`.github/workflows/ci.yml` の `test` ジョブが実行する内容と同一にする。
`build` ジョブはビルド可否のみを確認する（`cargo build --workspace --release`）。

### フロントエンド（Playwright E2E、任意）

```bash
# GUI動作確認（frontend/main.js を Tauri API モック経由で検証）
pnpm test:e2e
```

`pnpm lint` / `pnpm type-check` / `pnpm test` は存在しない。CIチェック
リストにもこれらは含めない。

---

## よくある失敗パターンと対処

| 失敗 | 原因 | 対処 |
|------|------|------|
| `cargo fmt --check` | フォーマット未適用 | `cargo fmt` を実行 |
| `cargo clippy` dead_code | 未使用フィールド | `#[allow(dead_code)]` または削除 |
| `cargo clippy` too_many_arguments | 引数8個以上 | `#[allow(clippy::too_many_arguments)]` |
| `cargo clippy` type_complexity | 複雑な型 | `#[allow(clippy::type_complexity)]` または型エイリアス定義 |
| `cargo clippy` vec_init_then_push | Vec::new()後に即push | `vec![...]` マクロに書き換え |
| `cargo test` が SIGSEGV/SIGABRT/STATUS_STACK_BUFFER_OVERRUN で異常終了 | 実 `ort::Session` 構築（`build_session`）に到達している。ONNX Runtime のプロセス終了時解放順序は ort crate 内部の実装詳細でCIから制御不能 | 該当テストに `#[ignore]` を付け、CI対象から外す（既知のケースは `model_service.rs` の `load_failure_invalid_onnx_bytes_never_produces_loaded_model` 等） |
| `cargo test` が別スレッド発のキャンセル/シグナルを待つテストで非決定的に失敗 | 進捗通知の送信と別スレッドでの状態変更（`request_cancel` 等）の間に単純な非同期チャネルしかなく、CI環境（共有CPU）でタイミングが逆転する | `mpsc::sync_channel(0)` でランデブー同期し、片方の操作が完了するまで他方をブロックする構造にする（確率で紛れさせず順序を決定的に保証する） |
| `cargo test` がテストの意図と無関係な assert で失敗（改名・シグネチャ変更後） | 実装をリネーム/変更した際に、文字列検査や期待値ベースのテスト（`generate_handler!` の登録名検査等）が追従していない | 実装側の現在の名称・シグネチャに合わせてテストの期待値を更新する |
| `pnpm tauri build` tauri not found | @tauri-apps/cli未インストール | `pnpm add -D @tauri-apps/cli`（現状 `tauri.conf.json` の `bundle.active` は `false` で、CIはバンドルせず素の実行バイナリのみ生成する） |
| アイコンファイル不在 | tauri.conf.jsonが存在しないファイルを参照 | bundle.icon を実在ファイルのみに修正 |

---

## バージョン更新対象ファイル

以下の3ファイルの `version` フィールドを同じ値に揃える:

1. `src-tauri/tauri.conf.json` → `"version": "X.Y.Z"`
2. `package.json` → `"version": "X.Y.Z"`
3. `Cargo.toml`（ワークスペースルート） → `[workspace.package]` セクション内の `version = "X.Y.Z"`

`src-tauri/Cargo.toml` は `version.workspace = true` によりルートの
`Cargo.toml` からバージョンを継承するのみで、直接編集する対象ではない。

### 注意事項

- **masterへのpushではリリースは作成されない。** `.github/workflows/release.yml`
  は `v*` タグの push でのみ起動する（`on.push.tags: ["v*"]`）。バージョンを
  上げてmasterへマージするだけではGitHub Releaseもリリースバイナリも作られない。
  下記「リリースタグの作成とpush」を必ず実行すること。
- リリースタグは `v{version}`（例: `v0.2.1`）。masterの対象コミットに対して付ける。
- 同じタグを再pushすると既存リリースが上書きされる（`softprops/action-gh-release`
  は同名タグのリリースを更新する）。
- バージョンを上げ忘れると前回リリースが上書きされるため、必ずタグ作成前に更新すること
- `Cargo.toml` のバージョンは `CARGO_PKG_VERSION` として MCP handshake の clientInfo 等で使用される。揃え忘れると外部プロセスに古いバージョンが通知される

### リリースタグの作成とpush

バージョン更新・マージ完了後、以下でタグを作成しpushする。これを行わない限り
`release.yml` は起動せずリリースバイナリは公開されない。

```bash
git checkout master
git tag -a v{version} -m "v{version}"
git push origin v{version}
```

タグpush後、GitHub Actions の `Release` ワークフローが Windows/Linux 向けの
素の実行バイナリ（`tag-editor-windows-x86_64.exe` / `tag-editor-linux-x86_64`。
インストーラではない）をビルドし、`RELEASE_NOTES.md` の内容を本文として
GitHub Release を作成・添付する。

---

## ドキュメント

### README.md（リポジトリルート）

`README.md` は「初めて訪れた利用者」向けの表紙。何ができるか・どう使うか・
使用技術・ライセンスを簡潔にまとめる。作成・再作成の指針はグローバルステアリング
`readme-guidelines.md`（README作成ガイドライン）に従う。

READMEには開発者向けの詳細（CIの全チェックコマンド、トラブルシュート、リリースの
全手順、内部アーキテクチャ）を書かない。それらはこの `project-ci-config.md` および
関連ステアリング・コード内に置き、READMEからは概要とリンクにとどめる。

機能追加や利用手順の変更を行った際は `README.md` も併せて更新する。

### このプロジェクトのREADMEに載せる主要項目

- **What**: 画像タグ付け（アノテーション）用デスクトップアプリであること
- **主な機能**: ONNX推論によるタグ自動付与、タグ編集・一括操作、仕訳/振分、改名/連番 等
  （利用者視点で簡潔に。Tag_Filter の全パラメータ列挙などの詳細は避ける）
- **使い方**: 起動して フォルダを開き、推論・編集・保存する基本フロー
- **使用技術**: Tauri v2 / Rust / ONNX Runtime（`ort` crate）/ 静的フロントエンド
- **ライセンス**: MIT

開発向けのビルド/テスト/リリース詳細はREADMEでは概要にとどめ、詳細は本ファイルを参照。

---

## リリースノートのフォーマット

`RELEASE_NOTES.md`（リポジトリルート）に記述する。`.github/workflows/release.yml`
の `release` ジョブが `body_path: RELEASE_NOTES.md` でこのファイルをそのまま
リリース本文として使う（YAML内に直接書き込む方式ではない）。

```markdown
# TagEditor vX.Y.Z

## 変更点

### 新機能
- [feat コミットから抽出]

### バグ修正
- [fix コミットから抽出]

### 改善
- [refactor/perf コミットから抽出]

## ダウンロード

GitHub Release から実行ファイルをダウンロードしてください。

- Windows: `tag-editor-windows-x86_64.exe`
- Linux: `tag-editor-linux-x86_64`
```

---

## CIチェックリスト

- [ ] `cargo fmt --check` 通過
- [ ] `cargo clippy --lib -- -D warnings` 通過
- [ ] `cargo test --workspace --release` 全通過
- [ ] `src-tauri/tauri.conf.json` の version 更新済み
- [ ] `package.json` の version 更新済み
- [ ] `Cargo.toml`（ワークスペースルート）の version 更新済み
- [ ] 3ファイルのバージョン一致
- [ ] `RELEASE_NOTES.md` 更新済み
- [ ] マージコミットメッセージに変更点含む
- [ ] masterへマージ・push済み
- [ ] `v{version}` タグを作成し push 済み（リリース作成の起動条件）
