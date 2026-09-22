# Implementation Plan: local-model-management

## Overview

本計画は design.md の Components and Interfaces / Data Models / Correctness Properties / Testing Strategy に厳密に沿って、既存 TagEditor（Tauri v2 + Rust、`ort` 推論タガー）のモデル管理を再設計し、タグフィルタとバッチ後タグ一覧を追加する破壊的変更を段階的に実装する。

積み上げ順は「純粋ロジック → サービス → コマンド境界 → アプリ配線 → UI」。まずランタイム非依存な純粋ロジック（`logic::tag_filter` / `logic::tag_batch` / `resolve_model_dir` / `variant_dir` / カタログ検証 / 存在判定）を実装・テストし、そのうえで既存の同期コア（`download_model` 系 / `load_local_model`）を `download_variant` / `load_variant` へ再編し、コマンド境界（`list_catalog` / `spawn_variant_download` / `start_inference` 拡張 / `overview_*`）を差し替え、最後に Tauri PathResolver 配線と frontend を接続する。

破壊的変更の対象（design.md「移行方針まとめ」）:

- `ModelLocation` 廃止 → `ModelSource { repo, onnx_file, tag_files }` 導入
- `ModelVariant.location` / `.onnx_available` 廃止 → `source: ModelSource` 追加
- `ModelFamily::Local` 廃止
- `builtin_remote_variants` / `list_models` / `scan_local_models` → `builtin_catalog` / `catalog_presence` / `resolve_model_dir` / `ensure_model_dir` / `variant_dir` へ再編
- `download_model(_with_progress)` → `download_variant`（同期コアの挙動を踏襲）
- `load_local_model` → `load_variant`（踏襲）
- `run_inference` の `threshold: f32` → `filter: TagFilter` へ拡張、`overview` を返す
- `adopt_by_threshold` → `logic::tag_filter::apply_filter` へ統合
- 旧テスト（`tests/pbt_model_onnx_filter.rs` の旧 Property 24 等）を新 Property へ差し替え

プロパティテストは `proptest` を用い最低 100 回反復。各テストにコメント `// Feature: local-model-management, Property {番号}: {本文}` を付す。

## Tasks

- [x] 1. データモデルの再設計（破壊的変更の土台）
  - [x] 1.1 `models.rs` のモデル定義を移行する
    - `src-tauri/src/models.rs` から `ModelLocation` enum を削除し、`ModelSource { repo: String, onnx_file: String, tag_files: Vec<String> }` を追加する（serde 対応）
    - `ModelFamily` から `Local` バリアントを削除し `Wd14` / `MlDanbooru` のみとする
    - `ModelVariant` から `location` / `onnx_available` を削除し `source: ModelSource` を追加する
    - `VariantPresence { variant: ModelVariant, present: bool }`、`CatalogListing { variants: Vec<VariantPresence>, excluded: Vec<ModelVariant>, model_dir_present: bool }` を追加する
    - `RawTagFilter` / `TagFilter` / `InvalidPattern` / `FilterOutcome` / `BatchOutcome` / `TagOverview` / `TagStat` を design.md の Data Models どおりに追加する（`TagFilter` は `regex::Regex` を保持するため serde 非導出、DTO 境界は `RawTagFilter` を用いる）
    - `InferBatchResult`（既存 `InferResult` を拡張）に `overview: TagOverview` を追加する。`Tag` / `LabelDef` / `Progress` / `LoadedModel` / `ChannelOrder` / `TagCategory` は変更しない
    - この時点でコンパイルは全体的に壊れる。以降のタスクで参照側を順次追従させる
    - _Requirements: 1.3, 2.1, 7.2, 9.1, 11.1_

- [x] 2. パス解決の純粋ロジック
  - [x] 2.1 `resolve_model_dir` / `variant_dir` を実装する
    - `src-tauri/src/services/model_service.rs`（または `logic` へ切り出し）に `resolve_model_dir(base_dir: &Path) -> PathBuf`（`base_dir` + 固定相対パス結合、決定的・絶対パスを返す）を実装する
    - `variant_dir(base_dir: &Path, variant: &ModelVariant) -> PathBuf`（`resolve_model_dir(base_dir)/<variant.id>`）を実装する
    - `ensure_model_dir(model_dir: &Path) -> AppResult<()>`（未作成なら作成、作成失敗・権限不足は `Io`/`AccessDenied`）を実装する
    - _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5_

  - [x] 2.2 `resolve_model_dir` の決定性と配下性のプロパティテスト
    - `tests/pbt_model_dir_resolve.rs` を新規作成
    - **Property 4: Model_Dir 解決の決定性と配下性**
    - **Validates: Requirements 2.1**
    - `base_dir` 生成器で空・末尾区切りあり/なし・非 ASCII をカバーし、同一入力で同一パス・`base_dir` を接頭辞に持つことを検証する

  - [x] 2.3 `variant_dir` の一意性のプロパティテスト
    - `tests/pbt_variant_dir_unique.rs` を新規作成
    - **Property 5: variant_dir の一意性**
    - **Validates: Requirements 2.3**
    - 相異なる `id` を持つ 2 つの `ModelVariant` で保存先パスが相異なることを検証する

- [x] 3. 静的カタログの構築と検証
  - [x] 3.1 `builtin_catalog` とカタログ検証を実装する
    - `model_service.rs` の `builtin_remote_variants` を削除し `builtin_catalog() -> Vec<ModelVariant>` を実装する。WD14 系各バリアント（`SmilingWolf/wd-vit-tagger-v3` 等、`source.onnx_file = "model.onnx"` 相当・`tag_files = ["selected_tags.csv"]`）と ML-Danbooru 系（`deepghs/ml-danbooru-onnx` の複数 `.onnx` を各 Variant として、`onnx_file` を各 `.onnx` パスに）を定義する
    - `validate_catalog(candidates) -> (Vec<ModelVariant>, Vec<ModelVariant>)`（登録集合・除外集合を返す純粋関数）を実装する。識別子・表示名・Model_Family のいずれか欠落/空を除外し、識別子重複は一意化する
    - `builtin_catalog` は `validate_catalog` を通した結果を返す
    - _Requirements: 1.3, 1.4, 1.5, 1.6, 1.7, 1.8_

  - [x] 3.2 カタログ必須フィールド非空のプロパティテスト
    - `tests/pbt_catalog_fields.rs` を新規作成
    - **Property 1: カタログ必須フィールドの非空**
    - **Validates: Requirements 1.3**

  - [x] 3.3 カタログ登録・除外の分割のプロパティテスト
    - 同ファイルに追加
    - **Property 2: カタログ登録・除外の分割**
    - **Validates: Requirements 1.4**
    - 空フィールド候補・欠落候補を含む生成器で、登録集合∪除外集合＝入力全体を検証する

  - [x] 3.4 識別子一意性のプロパティテスト
    - 同ファイルに追加
    - **Property 3: 識別子の一意性**
    - **Validates: Requirements 1.6, 1.7, 1.8**
    - 識別子衝突・同一リポジトリ複数 `.onnx` を含む生成器で、一意化後に重複が残らないことを検証する

  - [x] 3.5 旧 Property 24 テストを差し替える
    - `tests/pbt_model_onnx_filter.rs` を削除し、`.proptest-regressions`（`pbt_file_service_extensions.proptest-regressions` を除く該当があれば）を整理する
    - 旧 `list_models` / `ModelListing` / `onnx_available` / `ModelLocation` / `ModelFamily::Local` に依存する既存テストを列挙し、新 API へ差し替えまたは削除する
    - `builtin_catalog()` が WD14 系各バリアントと ML-Danbooru 複数 `.onnx` を個別 Variant として含むことの単体テストを追加する（要件 1.5, 1.6）
    - _Requirements: 1.5, 1.6_

- [x] 4. モデルのローカル存在判定
  - [x] 4.1 `catalog_presence` と存在判定を実装する
    - `model_service.rs` の `scan_local_models` / `local_variant` を削除する
    - `is_present(variant_dir: &Path) -> bool`（`.onnx` と `.csv`/`.json` タグ定義の対が揃うときのみ true、拡張子は大小無視）を実装する。既存 `onnx_with_tagdef` を再利用/整理する
    - `catalog_presence(base_dir: &Path, catalog: &[ModelVariant]) -> Vec<VariantPresence>` を実装する。Model_Dir 未作成なら全件 Not_Present
    - _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6_

  - [x] 4.2 Model_Present 判定の同値性のプロパティテスト
    - `tests/pbt_catalog_presence.rs` を新規作成
    - **Property 6: Model_Present 判定の同値性**
    - **Validates: Requirements 3.1, 3.2, 3.3, 3.4, 3.5, 3.6**
    - ファイル構成生成器で `.onnx` のみ・タグ定義のみ・両方・大文字拡張子（`.ONNX`/`.CSV`）・空ディレクトリ・ディレクトリ不在をカバーする

- [x] 5. Checkpoint - パス解決・カタログ・存在判定のテスト通過
  - `cargo test` でここまでの純粋ロジックのユニット/プロパティテストが通ることを確認する。Ensure all tests pass, ask the user if questions arise.

- [x] 6. タグフィルタの純粋ロジック（logic::tag_filter）
  - [x] 6.1 `compile_filter` を実装する
    - `src-tauri/src/logic/tag_filter.rs` を新規作成し `logic/mod.rs` に登録する
    - `compile_filter(raw: RawTagFilter) -> (TagFilter, Vec<InvalidPattern>)` を実装する。Exclude_Rules / Replace_Rules の検索パターンをタグ全体一致（`^...$`）・大小無視（`regex::RegexBuilder::case_insensitive(true)`）でコンパイルし、無効パターンは除外して `InvalidPattern` へ集める。Keep_Tags は「前後トリム＋大小無視」の正規化キー集合に変換する
    - _Requirements: 9.1, 9.7_

  - [x] 6.2 `apply_filter` を実装する
    - 同ファイルに `apply_filter(filter: &TagFilter, predicted: &[Tag]) -> FilterOutcome` を実装する。手順: (1) Replace を採用判定前に適用しタグ名を書換 → (2) 書換後名が Keep_Tags なら無条件 Adopted → (3) 非 Keep で Exclude 一致 or `confidence < threshold` なら Discarded → (4) それ以外 Adopted → (5) Additional_Tags を無条件 Adopted に追加。各タグは代表確信度算出のため confidence を保持する
    - 既存 `logic::inference_aux::adopt_by_threshold` を削除し、閾値採用ロジックを `apply_filter` へ統合する（`inference_aux` の他関数 `exclude_videos` / `resolve_batch_size` / `split_into_batches` は残す）
    - _Requirements: 9.2, 9.3, 9.4, 9.5, 9.6, 9.8_

  - [x] 6.3 Keep_Tags 無条件採用のプロパティテスト
    - `tests/pbt_tag_filter_keep.rs` を新規作成
    - **Property 11: Keep_Tags の無条件採用（Keep 優先）**
    - **Validates: Requirements 9.3, 9.8**

  - [x] 6.4 非 Keep タグの採用同値条件のプロパティテスト
    - `tests/pbt_tag_filter_adopt.rs` を新規作成
    - **Property 12: 非 Keep タグの採用同値条件**
    - **Validates: Requirements 9.2, 9.4**
    - confidence 境界（0.0/1.0/閾値ちょうど）・confidence なしタグをカバーする

  - [x] 6.5 Replace の採用判定前適用のプロパティテスト
    - `tests/pbt_tag_filter_replace.rs` を新規作成
    - **Property 13: Replace_Rules の採用判定前適用**
    - **Validates: Requirements 9.5**
    - replace 後の名前が keep/exclude に一致する書換ケースをカバーする

  - [x] 6.6 Additional_Tags 無条件付与のプロパティテスト
    - `tests/pbt_tag_filter_additional.rs` を新規作成
    - **Property 14: Additional_Tags の無条件付与**
    - **Validates: Requirements 9.6**

  - [x] 6.7 無効正規表現の非適用と通知のプロパティテスト
    - `tests/pbt_tag_filter_invalid_regex.rs` を新規作成
    - **Property 15: 無効な正規表現の非適用と通知**
    - **Validates: Requirements 9.7**
    - 無効パターン混在生成器で、`TagFilter` に無効パターンが残らず全て通知集合に入ることを検証する

- [x] 7. バッチ集計・一覧・書込の純粋ロジック（logic::tag_batch）
  - [x] 7.1 `apply_fraction_threshold` を実装する
    - `src-tauri/src/logic/tag_batch.rs` を新規作成し `logic/mod.rs` に登録する
    - `apply_fraction_threshold(per_image: &[FilterOutcome], filter: &TagFilter, image_count: usize) -> BatchOutcome` を実装する。各タグの出現割合（採用候補画像数 / バッチ対象画像数）を算出し、割合 < Fraction_Threshold かつ Keep/Additional でないタグを全画像の Adopted から Discarded へ移す。`fraction_threshold == 0` なら移送しない。`image_count == 1` なら非適用
    - _Requirements: 10.1, 10.2, 10.3, 10.4_

  - [x] 7.2 `build_overview` / `search_overview` を実装する
    - 同ファイルに `build_overview(batch: &BatchOutcome) -> TagOverview`（Adopted/Discarded 区別、各タグに代表確信度＝出現画像の confidence 平均、`image_count` 付き）を実装する
    - `search_overview(overview: &TagOverview, query: &str) -> TagOverview`（大小無視の部分一致で絞り込み）を実装する
    - _Requirements: 11.1, 11.2, 11.3_

  - [x] 7.3 Fraction_Threshold 採用同値条件のプロパティテスト
    - `tests/pbt_tag_batch_fraction.rs` を新規作成
    - **Property 17: Fraction_Threshold の採用同値条件**
    - **Validates: Requirements 10.1, 10.2, 10.3**
    - 全画像出現/一部出現・fraction 境界（0.0/割合ちょうど/1.0）・keep/additional 該当をカバーする

  - [x] 7.4 Fraction 単一画像非適用のプロパティテスト
    - 同ファイルに追加
    - **Property 18: Fraction の単一画像非適用**
    - **Validates: Requirements 10.4**

  - [x] 7.5 Tag_Overview 網羅と排他のプロパティテスト
    - `tests/pbt_tag_overview.rs` を新規作成
    - **Property 19: Tag_Overview の網羅と排他**
    - **Validates: Requirements 11.1**

  - [x] 7.6 代表確信度＝平均のプロパティテスト
    - 同ファイルに追加
    - **Property 20: 代表確信度＝出現確信度の平均**
    - **Validates: Requirements 11.2**

  - [x] 7.7 検索絞り込み述語一致のプロパティテスト
    - `tests/pbt_tag_overview_search.rs` を新規作成
    - **Property 21: Tag_Overview 検索絞り込みの述語一致**
    - **Validates: Requirements 11.3**
    - 空クエリ・大小混在・部分一致・非 ASCII をカバーする

- [x] 8. Checkpoint - タグフィルタ・バッチ集計のテスト通過
  - `cargo test` でフィルタ/バッチの純粋ロジックのテストが通ることを確認する。Ensure all tests pass, ask the user if questions arise.

- [x] 9. ダウンロードサービスの再編（download_variant）
  - [x] 9.1 `download_variant` へ再編する
    - `model_service.rs` の `download_model` / `download_model_with_progress` を `download_variant` として再編する。`ModelLocation::Remote` 分岐を廃し `variant.source.repo` / `variant.source.onnx_file` / `variant.source.tag_files` を用いる。`tag_definition_candidates` を `variant.source.tag_files` ベースへ置き換える
    - シグネチャを `download_variant(downloader, variant, variant_dir, cancel, on_progress) -> AppResult<PathBuf>` に統一する。既存の `fetch_with_retry`（30 秒 × 最大 3 回）・`atomic_write`（一時ファイル → リネーム）・部分ファイル除去・`DownloadPhase`・`DownloadError`・`ModelDownloader` トレイト・`HfHubDownloader` は挙動を変えず流用する
    - 新規（Not_Present 起点）失敗・キャンセルは部分ファイル除去。上書き（既存 Present）失敗時は既存 Assets を保持する分岐を実装する
    - _Requirements: 2.6, 5.4, 5.5, 5.6, 5.8, 6.1, 6.2, 6.3, 6.4, 6.5, 6.6_

  - [x] 9.2 保存から存在判定への整合のプロパティテスト
    - `tests/pbt_download_present.rs` を新規作成し、モック `ModelDownloader` + `tempfile` を用いる
    - **Property 7: 保存から存在判定への整合**
    - **Validates: Requirements 5.5**
    - 成功後に `is_present(variant_dir)` が true になることを検証する

  - [x] 9.3 原子的保存の全か無かのプロパティテスト
    - `tests/pbt_download_atomic.rs` を新規作成
    - **Property 8: 原子的保存の全か無か**
    - **Validates: Requirements 2.4, 2.6, 4.4, 5.4, 5.6, 6.4, 6.5, 6.6**
    - Download 結末生成器で onnx 取得失敗・タグ定義取得失敗・保存失敗・キャンセル・成功をカバーし、失敗/キャンセル時に Model_Present な Assets が残らないことを検証する

  - [x] 9.4 取得の再試行上限のプロパティテスト
    - `tests/pbt_download_retry.rs` を新規作成
    - **Property 9: 取得の再試行上限**
    - **Validates: Requirements 6.1, 6.2**
    - 「k 回目で初成功/全失敗」モックで `fetch` 呼び出しが 3 回を超えず、`k<=3` は成功・`k>3` は 3 回打ち切り失敗となることを検証する（k = 1, 3, 4, ∞ の境界）

  - [x] 9.5 上書き失敗時の既存 Assets 保持のプロパティテスト
    - `tests/pbt_download_overwrite.rs` を新規作成
    - **Property 10: 上書き失敗時の既存 Assets 保持**
    - **Validates: Requirements 5.8**
    - 既に Model_Present な `variant_dir` に上書き失敗させ、上書き前の対が保持され Model_Present のままであることを検証する

- [x] 10. モデル読込サービスの再編（load_variant）
  - [x] 10.1 `load_variant` へ再編する
    - `model_service.rs` の `load_local_model` を `load_variant(variant_dir: &Path) -> AppResult<LoadedModel>` にリネームし挙動を踏襲する。`parse_tag_definition` / `find_tag_definition` / `build_session` は流用する
    - `ModelFamily::Local` 削除に伴う `tag_definition_candidates` の防御分岐など不要コードを整理する
    - 既存 `load_local_model` の異常系ユニットテスト（ONNX 読込不可・タグ定義欠落/不正 → `ModelLoad`）を `load_variant` へ追従させる
    - _Requirements: 8.1, 8.2, 8.3, 8.4_

- [x] 11. 推論オーケストレーションの拡張（run_inference）
  - [x] 11.1 `run_inference` を filter/overview 対応へ拡張する
    - `src-tauri/src/services/inference_service.rs` の `run_inference` の引数 `threshold: f32` を `filter: &TagFilter` へ変更する。各画像で `logic::tag_filter::apply_filter` を適用して `FilterOutcome` を得て、バッチなら `logic::tag_batch::apply_fraction_threshold` を適用し、最終 Adopted_Tags を同名 Tag_File へ `render_tags` + `tag_file::write_tag_file` で書込む
    - 戻り値を `InferBatchResult`（`overview: TagOverview` を含む）にし、`logic::tag_batch::build_overview` で構築する。mp4 除外・個別失敗スキップ・進捗・キャンセル・rayon 並列前処理は不変
    - `adopt_by_threshold` 参照を削除する
    - _Requirements: 4.4, 7.1, 7.2, 7.4, 9.9, 10.1, 10.4, 11.1_

  - [x] 11.2 Adopted_Tags の Tag_File 書込一致のプロパティテスト
    - `tests/pbt_tag_file_adopted.rs` を新規作成
    - **Property 16: Adopted_Tags の Tag_File 書込一致**
    - **Validates: Requirements 9.9**
    - `FilterOutcome` を `tempfile` の Tag_File へ書込→読込し、Adopted をカンマ＋スペース連結した文字列と一致し Discarded を含まないことを検証する

- [x] 12. Checkpoint - サービス層のテスト通過
  - `cargo test` で download/load/inference サービスのテストが通ることを確認する。Ensure all tests pass, ask the user if questions arise.

- [x] 13. コマンド境界の差し替え
  - [x] 13.1 `list_catalog` を実装する
    - `src-tauri/src/commands/adapters.rs` の `list_models` を削除し `list_catalog(base_dir: Option<String>) -> AppResult<CatalogListing>` を実装する。`builtin_catalog()` と `catalog_presence(base_dir, &catalog)` を結合し、`validate_catalog` の除外集合と `model_dir_present` を返す
    - _Requirements: 1.1, 1.2, 3.1, 3.5_

  - [x] 13.2 `spawn_variant_download` を実装する
    - `spawn_model_download` / `spawn_model_download_core` を `spawn_variant_download(variant_id, operation_id)` へ再編する。`variant_id` を `builtin_catalog()` から解決し、`variant_dir(base_dir, &variant)` を保存先に `download_variant` を別スレッド起動する。進捗 emit（`PROGRESS_EVENT`、`DownloadPhase` → `Progress` 写像）とキャンセル（`CancelRegistry`）は踏襲する
    - _Requirements: 5.1, 5.2, 5.3, 5.4, 5.5_

  - [x] 13.3 `start_inference` を拡張する
    - `start_inference` / `start_inference_core` の引数に `variant_id: String` と `filter: RawTagFilter` を追加し、旧 `threshold` を廃す。`compile_filter` で `TagFilter` を得て（無効パターンは `InvalidInput` として UI へ通知）、`variant_dir` を解決し `is_present` 判定する
    - Not_Present なら `download_variant`（遅延 DL、進捗通知）→ 失敗時は推論せずエラー。Present ならそのまま `load_variant`。読込失敗/DL 失敗は推論しない。ローカルに Assets が無い場合は `NotFound` を返し推論しない。ロードした `LoadedModel` を runner として `run_inference` に `TagFilter` を渡す
    - _Requirements: 4.1, 4.2, 4.3, 4.4, 4.5, 7.1, 7.3, 7.5_

  - [x] 13.4 Tag_Overview コマンド群を実装する
    - `adapters.rs` に `overview_search(query)`（`search_overview` 委譲）、`overview_send_keep(tags)` / `overview_send_exclude(tags)`（現行 Keep_Tags / Exclude_Rules へ追加）、`rerun_inference(...)`（更新後 filter で同一バッチへ `apply_filter` / `apply_fraction_threshold` / `build_overview` を再適用し一覧反映）を実装する
    - _Requirements: 11.3, 11.4, 11.5, 11.6_

  - [x] 13.5 keep/exclude 送出後の包含のプロパティテスト
    - `tests/pbt_overview_send.rs` を新規作成
    - **Property 22: keep/exclude 送出後の包含**
    - **Validates: Requirements 11.4, 11.5**

  - [x] 13.6 遅延 DL 配線とローカルのみ実行の統合テスト
    - `tests/smoke_app.rs` またはモジュール内テストに追加（モック `ModelDownloader` / `MockRunner` 使用、各 1〜3 例）
    - Not_Present → `download_variant` → `load_variant` → `run_inference` の順序、Present なら `download_variant` を呼ばないこと（要件 4.1, 4.2, 4.3）
    - `is_present` が false のとき推論前段で `NotFound`（要件 7.3, 7.5）
    - `run_inference` が runner のみを用い Model_Source を参照しない（型設計上コンパイル時担保、要件 7.1, 7.2, 7.4）
    - 再推論反映: Keep_Tags 更新後に同一バッチへ再適用すると追加タグが Adopted 側へ移る（要件 11.6）
    - _Requirements: 4.1, 4.2, 4.3, 7.1, 7.2, 7.3, 7.4, 7.5, 11.6_

- [x] 14. アプリ配線（Tauri PathResolver で base_dir 供給）
  - [x] 14.1 コマンド登録と base_dir 供給を配線する
    - `src-tauri/src/app.rs` の `generate_handler!` から旧 `list_models` / `download_model` / `load_local_model` / `spawn_model_download` を外し、`list_catalog` / `spawn_variant_download` / 拡張 `start_inference` / `overview_search` / `overview_send_keep` / `overview_send_exclude` / `rerun_inference` を登録する
    - Tauri v2 の PathResolver（`app.path().resource_dir()` 等、install 基準ディレクトリ）から `base_dir` を取得して各コマンドへ供給する経路を配線する。純粋関数 `resolve_model_dir` にはこの実 `base_dir` を渡す
    - `ensure_model_dir` を Download_Operation のデータ書込前に呼ぶ配線を確認する（要件 2.2）
    - _Requirements: 2.1, 2.2_

- [x] 15. Checkpoint - Rust 全体のビルドとテスト通過
  - `cargo build` と `cargo test` が通ること、旧 API 参照が残っていないことを確認する。Ensure all tests pass, ask the user if questions arise.

- [x] 16. フロントエンド配線（Model_Management_Tab）
  - [x] 16.1 カタログ一覧・存在状態・DL ボタン・進捗を配線する
    - `frontend/main.js` の `refreshModels` / `list_models` 呼び出しを `list_catalog`（`base_dir` は Rust が解決するため引数不要化）へ差し替え、`state.models` を `CatalogListing`（`variants: VariantPresence[]` / `excluded` / `model_dir_present`）に合わせる。ローカルディレクトリ選択 UI（`modelLocalDir` / `modelLocalPick` / `modelLoadLocal`）は固定 Model_Dir 方針に合わせて撤去/無効化する
    - 各 Variant 行に Model_Present / Not_Present 表示とダウンロードボタンを描画し、`spawn_variant_download`（`variant_id` + `operation_id`）を呼ぶ。`inference://progress` 購読でダウンロード進捗を 0〜100% 表示、キャンセルボタンで `cancel_operation` を呼ぶ
    - `frontend/index.html` にカタログ一覧・存在バッジ・DL/キャンセルボタン・進捗バーの要素を追加、`frontend/styles.css` に対応スタイルを追加する
    - _Requirements: 1.1, 1.2, 3.1, 3.5, 5.1, 5.3_

- [x] 17. フロントエンド配線（Tag_Filter パネル）
  - [x] 17.1 Tag_Filter 入力 UI を配線する
    - `frontend/index.html` / `styles.css` に keep / exclude / replace（検索→置換の対）/ additional / confidence_threshold / fraction_threshold の入力要素を追加する
    - `frontend/main.js` の推論起動（`start_inference`）で旧 `threshold` の代わりに `RawTagFilter` DTO を組み立てて `variant_id` と共に渡す。無効正規表現通知（`InvalidInput`）を UI に表示する
    - _Requirements: 9.1, 9.7_

- [x] 18. フロントエンド配線（Tag_Overview）
  - [x] 18.1 バッチ後タグ一覧 UI を配線する
    - `frontend/index.html` / `styles.css` に Tag_Overview 表示（採用/不採用 2 区分・タグ名＋代表確信度・検索ボックス・keep/exclude 送出ボタン）を追加する
    - `frontend/main.js` で `start_inference` 完了時に受け取る `InferBatchResult.overview` を 2 区分描画し、検索は `overview_search`、送出は `overview_send_keep` / `overview_send_exclude`、再推論は `rerun_inference` を呼んで一覧を更新する
    - _Requirements: 11.1, 11.2, 11.3, 11.4, 11.5, 11.6_

- [x] 19. 最終 Checkpoint - 全テスト通過と手動確認
  - `cargo build` / `cargo test` を実行し全テスト通過を確認する。生成した一時ファイルを掃除する。UI（一覧表示・進捗バー・2 区分表示）は snapshot/手動確認とする。Ensure all tests pass, ask the user if questions arise.

## Notes

- `*` 付きサブタスクは任意（テスト系）。ただし本 spec では Correctness Properties が検収の中核のため、対応する `*` サブタスクは可能な限り実装する
- 各タスクは特定の要件番号を参照し、該当する場合はプロパティ番号を明記している
- プロパティテストは `proptest` を用い最低 100 回反復。コメント形式は `// Feature: local-model-management, Property {番号}: {本文}`
- 実ネットワークダウンロード・実 ONNX 推論・スレッド/進捗/キャンセル配線は統合テストで 1〜3 例に絞る（design.md Testing Strategy）。UI 表示は PBT 非対象で snapshot/手動確認
- Task 1 でモデル定義を破壊的に変更するため一時的に全体がビルド不能になる。以降のタスクで参照側を順に追従させ、Task 15 で全体のビルド/テスト通過を担保する

## Task Dependency Graph

```json
{
  "waves": [
    { "id": 0, "tasks": ["1.1"] },
    { "id": 1, "tasks": ["2.1", "6.1", "7.1"] },
    { "id": 2, "tasks": ["3.1", "2.2", "2.3", "6.2", "7.2"] },
    { "id": 3, "tasks": ["4.1", "3.2", "3.3", "3.4", "3.5", "6.3", "6.4", "6.5", "6.6", "6.7", "7.3", "7.4", "7.5", "7.6", "7.7"] },
    { "id": 4, "tasks": ["9.1", "4.2"] },
    { "id": 5, "tasks": ["10.1", "9.2", "9.3", "9.4", "9.5"] },
    { "id": 6, "tasks": ["11.1"] },
    { "id": 7, "tasks": ["13.1", "11.2"] },
    { "id": 8, "tasks": ["13.2"] },
    { "id": 9, "tasks": ["13.3"] },
    { "id": 10, "tasks": ["13.4"] },
    { "id": 11, "tasks": ["13.5", "13.6", "14.1"] },
    { "id": 12, "tasks": ["16.1", "17.1", "18.1"] }
  ]
}
```
