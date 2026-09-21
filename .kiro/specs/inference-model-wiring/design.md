# 推論・モデル結線（inference-model-wiring）Bugfix Design

## Overview

本 bugfix は、タグ推論バッチ機能とプレビュー/サムネイル表示について、UI 層（`frontend/main.js`）と Rust コマンド層（`src-tauri/src/commands/adapters.rs`, `src-tauri/src/app.rs`, `src-tauri/src/services/*`）の結線・実装欠落に起因する 4 系統の不具合を修正する。

不具合は次の 2 グループに分かれる。

- **グループ A（タスク 21 の UI-Rust 結線未完に起因、不具合 1〜3）**: バッチ推論の起動コマンドが未定義・未登録、選択モデルの `ort::Session` をアプリ層で保持する経路が不在、モデルダウンロードが同期実行で進捗・キャンセル未配線。
- **グループ B（プレビュー/サムネイル転送方式に起因、不具合 4）**: PNG バイト列（`Vec<u8>`）を JSON 数値配列として IPC 転送しており、サイズ膨張・手動 Base64 化・Rust 側キャッシュ不在により表示遅延が大きい。

修正方針は既存 `tag-editor` spec の設計方針を踏襲する。長時間処理は別スレッドで実行し進捗をイベント push、キャンセルは `CancelRegistry` 経由、`ort::Session` は serde 不可のためアプリ層状態管理（`tauri::State`）に保持する。既存の同期コア（`run_inference_job` / `spawn_inference_job` / `download_model` / `get_preview` / `get_thumbnail`）とテスト済み挙動は変えず、**未配線部分の結線と転送方式の追加のみ**で修正する。

## Glossary

- **Bug_Condition (C)**: バグを誘発する入力条件。本 bugfix では「推論起動要求」「モデル選択/ダウンロード後のロード要求」「モデルダウンロード要求」「プレビュー/サムネイル表示要求」の 4 経路が該当する。
- **Property (P)**: C に対する期待挙動。推論・ダウンロードが実際に起動し進捗・キャンセルが機能すること、選択モデルが推論から参照可能になること、プレビュー/サムネイルが JSON 数値配列転送を経ずに表示されること。
- **Preservation**: C に該当しない入力に対し、既存挙動・戻り値・エラー種別が変わらないこと。特に既存サービスコマンド群、`run_inference`/`download_model` の同期コア、`ModelListing`/`LoadedModelInfo`/`PreviewData`/`ThumbnailData` の DTO 形状。
- **`spawn_inference_job`**: `src-tauri/src/commands/adapters.rs` の別スレッド起動関数。`run_inference_job`（同期コア）を `std::thread` 上で走らせ `JoinHandle` を返す。
- **`run_inference_job`**: 同 Tauri 非依存の同期コア。`CancelRegistry::register` でフラグ登録、`ProgressEmitter` で進捗通知、完了時に `clear`。
- **`SessionRunner` / `OrtSessionRunner<'a>`**: 推論実行の抽象トレイトと ort 実装。`OrtSessionRunner` は `&'a mut ort::Session` を借用して構築する。
- **`LoadedModel`**: `ort::Session` + `input_size` + `channel_order` + `labels` を保持する非 serde 構造体（`src-tauri/src/models.rs`）。
- **`CancelRegistry`**: `operation_id` → `Arc<AtomicBool>` のキャンセルフラグを管理する `tauri::State`。`app.rs` で `manage` 済み。
- **`TauriProgressEmitter`**: `AppHandle` を包み `PROGRESS_EVENT = "inference://progress"` へ進捗を emit するエミッタ。
- **ModelSessionState**: 本 bugfix で追加する、ロード済み `LoadedModel` を保持するアプリ層状態管理（`tauri::State`）。
- **Asset Protocol**: Tauri の `asset:`/`convertFileSrc` によるローカルファイル直接参照。`tauri.conf.json` で `assetProtocol.enable=true`、`scope=["**"]` 済み。

## Bug Details

### Bug Condition

不具合は、UI が発行する 4 種の要求のいずれかを処理する経路で、必要な Rust 側の起動/保持/転送機構が欠落している状態として現れる。

**形式仕様:**

```
FUNCTION isBugCondition(input)
  INPUT: input of type UiRequest
  OUTPUT: boolean

  // 1) 推論起動: operation_id を発行し進捗購読を張るが、
  //    バッチ推論を起動する #[tauri::command] が呼ばれない
  RETURN (input.kind == StartInference
            AND NOT inferenceCommandInvoked(input))

  //    または 2) モデル選択/ダウンロード後のロード:
  //    選択/取得済みモデルを ort::Session として保持・参照する経路が無い
       OR (input.kind IN [SelectModel, ModelDownloaded]
            AND NOT loadedSessionRetrievableAtInference(input))

  //    または 3) モデルダウンロード: 同期実行かつ進捗/キャンセル未配線
       OR (input.kind == DownloadModel
            AND (downloadRunsSynchronously(input)
                 OR NOT downloadProgressSubscribed(input)))

  //    または 4) プレビュー/サムネイル: PNG バイト列を JSON 数値配列で転送
       OR (input.kind IN [GetPreview, GetThumbnail]
            AND transferredAsJsonNumberArray(input))
END FUNCTION
```

### Examples

- 推論タブで実行ボタンを押す → UI は `operation_id="infer-<ts>"` を払い出し `inference://progress` を購読するが、Rust の推論は一切呼ばれず進捗バーが 0 のまま停止する（期待: バッチ推論が起動し進捗が更新される）。
- モデルタブでローカルモデルを読み込む → `load_local_model` はメタ情報（`LoadedModelInfo`）を返すのみで、`ort::Session` はドロップされる。推論実行時に参照する手段が無い（期待: セッションがアプリ層に保持され推論から参照可能）。
- リモートモデルをダウンロードする → `download_model` が同期でネットワーク完了までブロックし、進捗・キャンセルのフィードバックが無い（期待: 別スレッド起動・進捗イベント・`cancel_operation` による中断）。
- 大きい画像を選択する → `get_preview` の PNG（例 数 MB）が JSON 数値配列（1 バイト＝数文字）に膨張して転送され、`pngBytesToDataUrl` の手動 Base64 ループが走り、表示まで待たされる（期待: 数値配列転送と手動 Base64 を経ない表示）。
- 同一画像を再選択する → Rust 側キャッシュが無く、毎回ディスク読込・デコード・リサイズ・PNG 再エンコードが走る（期待: 再エンコードの繰り返し削減）。

## Expected Behavior

### Preservation Requirements

**Unchanged Behaviors（bugfix.md 3.1〜3.10 に対応）:**

- 既存サービスコマンド群（`list_images`, `bulk_add_tags`, `sort_files`, `rename_regex`, `find_orphan_captions`, `capabilities`, `create_symlink`, `convert_path` 等）の挙動・戻り値・エラー種別を維持する（3.1）。
- `run_inference` / `run_inference_job` / `spawn_inference_job` の同期コアの検証済み挙動（進捗の単調性・最終値、キャンセル反映、部分失敗時の処理前状態保持）を維持する（3.2）。
- `list_models` のローカル/リモート一覧と ONNX 非対応の除外表示（`available`/`excluded`）を維持する（3.3）。
- `load_local_model` が返す `LoadedModelInfo`（`input_size`/`label_count`）の形式を維持する（3.4）。
- `cancel_operation` の `CancelRegistry` 経由挙動（登録済み `true` / 未登録 `false`）を維持する（3.5）。
- 非 Windows で `capabilities` が `windows_only: false` を返す挙動を維持する（3.6）。
- プレビュー/サムネイル取得失敗時の `placeholder=true` 代替表示と他項目への非波及を維持する（3.7）。
- サムネイルサイズの `MIN_THUMBNAIL_SIZE(64)`〜`MAX_THUMBNAIL_SIZE(512)` クランプとアスペクト比保持縮小（拡大なし）を維持する（3.8）。
- プレビューの `MAX_PREVIEW_SIZE(2048)` 超のみ内接縮小・以下は元寸法という規約を維持する（3.9）。
- `PreviewData` / `ThumbnailData` の既存フィールド形状（`width`/`height`/`png`/`placeholder`）を維持する（3.10）。

**Scope:**

Bug_Condition に該当しない入力は本 bugfix の影響を受けない。特に以下は完全に不変とする。

- 既存サービスコマンドの呼び出しと戻り値。
- 同期コア関数の直接呼び出し（テストが検証する内部 API）。
- 既存 DTO の JSON 形状（フィールド追加は後方互換を壊さない範囲でのみ許容し、既存フィールドは削除・改名しない）。

期待される正しい挙動そのものは Correctness Properties（Property 1）で定義する。本節は「変えてはならないもの」を規定する。

## Hypothesized Root Cause

1. **推論起動コマンドの欠落と未登録（不具合 1）**: `run_inference_job`/`spawn_inference_job` は実装・テスト済みだが、これを呼ぶ `#[tauri::command]` が存在せず、`app.rs` の `generate_handler!` にも未登録。UI の `ops.inferRun` は `operation_id` 発行と進捗購読のみで `invoke` しない（`main.js` のコメントにも TODO として明記）。

2. **モデルセッション状態管理の不在（不具合 2）**: `load_local_model` コマンドは `LoadedModel` の `ort::Session` をドロップし `LoadedModelInfo` だけ返す。`LoadedModel` は serde 不可のため UI に返せず、アプリ層で保持する `tauri::State` が存在しない。よって推論実行時に `OrtSessionRunner` を構築するためのセッション参照手段が無い。リモートモデルはダウンロード後のロード呼び出し自体が未配線。

3. **ダウンロードの同期実行と進捗/キャンセル未配線（不具合 3）**: `download_model` コマンドが `model_service::download_model`（同期・再試行付き）を直接呼びブロックする。`spawn_download_job` は未実装で、進捗通知・`CancelRegistry` 連携が無い。UI も進捗購読なしに `await` する。

4. **プレビュー/サムネイルの転送方式（不具合 4）**: `PreviewData.png`/`ThumbnailData.png` が `Vec<u8>` で、Tauri IPC は JSON 数値配列にシリアライズする。フロントは `pngBytesToDataUrl` でチャンク単位の手動 Base64 化を行う。`assetProtocol` は有効化済みだが未使用。Rust 側キャッシュも無く再エンコードが繰り返される。

## Correctness Properties

Property 1: Bug Condition - 結線後に各要求が実際に起動・保持・軽量転送される

_For any_ input where the bug condition holds（`isBugCondition` が true を返す）, the fixed system SHALL 次を満たす:
(a) 推論起動要求では、選択モデル・対象画像・閾値・Batch_Size を用いて `spawn_inference_job` 経由でバッチ推論をバックグラウンドスレッドで起動する `#[tauri::command]` が実際に呼ばれ、発行済み `operation_id` で `inference://progress` が送出され進捗が更新され、完了時にキャンセルレジストリが解放される。モデル未ロード時はクラッシュせず明確なエラーを返す。
(b) モデル選択/ダウンロード後のロード要求では、ロードした `LoadedModel`（`ort::Session` を含む）をアプリ層状態管理に保持し、以後の推論から参照可能にする。
(c) モデルダウンロード要求では、処理を別スレッドで spawn して呼び出し元をブロックせず、進捗をイベント通知し、`cancel_operation`/`CancelRegistry` で中断できる。
(d) プレビュー/サムネイル表示要求では、PNG バイト列を JSON 数値配列化する経路を用いず（asset protocol による直接参照）、フロントの手動 Base64 化を経ずに表示し、同一画像の再表示で不要な再デコード・再エンコードを削減する。

**Validates: Requirements 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 2.8, 2.9, 2.10, 2.11, 2.12**

Property 2: Preservation - 非該当入力の既存挙動と DTO 形状の不変性

_For any_ input where the bug condition does NOT hold（`isBugCondition` が false）, the fixed system SHALL produce the same result as the original system, preserving 既存サービスコマンドの挙動・戻り値・エラー種別、同期コア（`run_inference`/`run_inference_job`/`spawn_inference_job`/`download_model`）の検証済み挙動、`cancel_operation` の登録済み/未登録の戻り値、`capabilities` のプラットフォーム分岐、プレビュー/サムネイル失敗時の `placeholder` 代替表示・サイズクランプ・縮小規約、および `ModelListing`/`LoadedModelInfo`/`PreviewData`/`ThumbnailData` の既存フィールド形状。

**Validates: Requirements 3.1, 3.2, 3.3, 3.4, 3.5, 3.6, 3.7, 3.8, 3.9, 3.10**

## Fix Implementation

### 前提（変えない中核）

`run_inference_job` / `spawn_inference_job` / `run_inference` / `download_model`（同期コア）/ `get_preview` / `get_thumbnail` の関数本体は変更しない。本 bugfix は **アプリ層の結線（新コマンド・State・登録）とフロントの呼び出し・転送方式** を追加する。

### 変更 1: アプリ層モデルセッション状態管理（不具合 2 の基盤）

**File**: `src-tauri/src/app.rs`（新規 State）、必要なら `src-tauri/src/commands/` に補助モジュール。

- ロード済みモデルを保持する State を追加する。`LoadedModel` は非 serde・非 `Sync`（`ort::Session` の `run` が `&mut` 前提で内部可変性を持つ）ため、`Mutex` で包む。単一選択モデル運用なら `Mutex<Option<LoadedModel>>`、複数保持ならモデル `id` をキーにした `Mutex<HashMap<String, LoadedModel>>`（ModelSessionState）を採用する。まずは単一保持で最小結線し、必要に応じてキー付きへ拡張する。
- `app.rs` の `Builder` に `.manage(ModelSessionState::default())` を追加する。

```
// 擬似コード（app.rs）
struct ModelSessionState {
    current: Mutex<Option<LoadedModel>>,   // 選択中モデルの実セッション
}
builder.manage(ModelSessionState::default());
```

### 変更 2: モデルロードでセッションを保持する（不具合 2、期待 2.4/2.5）

**File**: `src-tauri/src/commands/adapters.rs`

- `load_local_model` コマンドを、`ModelSessionState` を `tauri::State` で受け取り、`model_service::load_local_model` が返す `LoadedModel` を State に格納する形へ拡張する。**戻り値の `LoadedModelInfo`（`input_size`/`label_count`）形状は維持する**（保持 3.4）。
- リモートモデルは、ダウンロード完了後の保存先ディレクトリを `model_service::load_local_model` に渡してロードし、同じ State に保持する経路を追加する（期待 2.5）。ダウンロードコマンド完了 → ロードの二段、または「ダウンロード後に自動ロード」オプションで結線する。

```
// 擬似コード
#[tauri::command]
fn load_local_model(state: State<ModelSessionState>, dir: String) -> AppResult<LoadedModelInfo> {
    let loaded = model_service::load_local_model(Path::new(&dir))?;
    let info = LoadedModelInfo { input_size: loaded.input_size, label_count: loaded.labels.len() };
    *state.current.lock().unwrap() = Some(loaded);  // セッションを保持
    Ok(info)  // 既存 DTO 形状は不変
}
```

### 変更 3: バッチ推論を起動する新コマンド（不具合 1、期待 2.1/2.2/2.3/2.6）

**File**: `src-tauri/src/commands/adapters.rs`（新コマンド）、`src-tauri/src/app.rs`（登録）

- 新 `#[tauri::command] start_inference` を追加する。引数は `image_paths`/`threshold`/`batch_size`/`operation_id` と、State（`ModelSessionState`, `CancelRegistry`）・`AppHandle`。
- セッション未ロードなら `AppError`（モデル未選択/未ロード）を返しクラッシュしない（期待 2.6）。
- ロード済み `LoadedModel` から `labels`/`input_size`/`channel_order` を読み取り `InferenceJob` を組み立て、`OrtSessionRunner`（`&mut ort::Session` を借用）を構築して `spawn_inference_job` を起動する。進捗は `TauriProgressEmitter`（`Send`）で `inference://progress` に emit。

**設計上の注意（`Send` とスレッド move）**: `spawn_inference_job<R: SessionRunner + Send + 'static>` は runner をスレッドへ move する。`OrtSessionRunner<'a>` は `&'a mut Session` を借用するライフタイム付きで `'static` を満たさない。よって次のいずれかで整合させる:

- (推奨) `LoadedModel` を State から **take して所有権ごとスレッドへ move** し、スレッド内で `OrtSessionRunner::new(&mut model.session, input_size)` を構築して `run_inference_job` を直接呼ぶ（`spawn_inference_job` の総称境界に載せる代わりに、所有 `LoadedModel` を包む `Send` な runner を用意する）。完了後にモデルを State へ戻すか、`Arc<Mutex<Option<LoadedModel>>>` を move して内部で借用する。
- あるいは所有 `Session` を持つ `Send` な runner 型（`OwnedOrtRunner { model: LoadedModel }`）を新設し `SessionRunner` を実装する。これなら `'static + Send` を満たし `spawn_inference_job` にそのまま渡せる。

この runner 追加は同期コアを変えず、`SessionRunner` 実装の追加のみで済む（保持 3.2）。

- `app.rs` の `generate_handler!` に `start_inference`（と後述 `spawn_model_download`）を追加登録する。

```
// generate_handler! への追加（app.rs）
crate::commands::adapters::start_inference,
crate::commands::adapters::spawn_model_download,
```

### 変更 4: モデルダウンロードの spawn・進捗・キャンセル（不具合 3、期待 2.7/2.8/2.9）

**File**: `src-tauri/src/commands/adapters.rs`（新コマンド + spawn 補助）、`src-tauri/src/services/model_service.rs`（進捗コールバック引数の追加は任意）

- 新 `#[tauri::command] spawn_model_download` を追加する。引数は `variant: ModelVariant`/`dest_dir`/`operation_id` と State（`CancelRegistry`）・`AppHandle`。
- 別スレッドで `model_service::download_model`（同期コア・不変）を実行し、即座に戻る。進捗は「ファイル取得の段階（onnx 取得 → タグ定義取得 → 保存）」に応じて `inference://progress`（または専用イベント名）で通知する。キャンセルは `CancelRegistry::register` したフラグを取得段の境界で確認して中断する。
- `download_model`（既存同期コマンド）は互換のため残すか、`spawn_model_download` へ委譲する。既存の再試行・原子的保存・部分ファイル除去（15.7/15.8）の挙動は変えない（保持 3.4 周辺）。
- 進捗をコアに通知させる場合は `download_model` に `progress: &mut dyn FnMut(...)` を追加するが、既存テストの呼び出しシグネチャを壊さないよう、オーバーロード相当の薄いラッパ（`download_model_with_progress`）を新設し、既存 `download_model` は据え置く。

### 変更 5: プレビュー/サムネイル転送の再設計（不具合 4、期待 2.10/2.11/2.12）

**File**: `frontend/main.js`（asset protocol 利用）、`src-tauri/src/services/file_service.rs`（キャッシュ、任意）、`src-tauri/src/commands/adapters.rs`（キャッシュ経路のコマンド、任意）

方針は 2 案を併記し、design レビューで確定する。

- **案 A（推奨・低コスト）: Asset Protocol 直接参照**。`assetProtocol.enable=true`/`scope=["**"]` は設定済み。サムネイル/プレビューが「元寸法で足りる」または「縮小生成後のファイルパスを返せる」場合、フロントは `convertFileSrc(path)` で `<img src>` に直接ローカルパスを割り当て、JSON バイト列転送と `pngBytesToDataUrl` を完全に排除する。
  - サムネイルはグリッド表示で縮小が必須のため、Rust 側で生成した縮小 PNG を一時ディレクトリ（アプリキャッシュ）へ書き出し、そのパスを返す新経路（`get_thumbnail_path`/`get_preview_path`）を追加する。フロントはそのパスを `convertFileSrc` で参照する。
  - プレビューは `MAX_PREVIEW_SIZE` 以下なら元ファイルを直接 `convertFileSrc` 参照、超過時のみ縮小ファイルのパスを返す。
- **案 B（後方互換・段階移行）: 既存バイナリ経路は残しつつキャッシュを追加**。`PreviewData`/`ThumbnailData` の形状は不変（保持 3.10）とし、Rust 側に `(path, size, mtime)` をキーにした LRU 等の簡易キャッシュを設け、同一画像の再デコード・再リサイズ・再エンコードを回避する（期待 2.12）。転送量そのものは案 A ほど下がらないため、案 A と併用するのが望ましい。

いずれの案でも:
- 失敗時の `placeholder` 代替表示（3.7）と、サイズクランプ・縮小規約（3.8/3.9）は現行ロジックを流用して不変に保つ。
- 既存 `get_preview`/`get_thumbnail` と DTO は削除せず、新経路を **追加** する（保持 3.10）。フロントは新経路を優先しつつ、失敗時は `placeholder` にフォールバックする。

### 変更 6: フロントエンドの結線（`frontend/main.js`）

- `ops.inferRun` ハンドラで `operation_id` 発行・進捗購読の後、`invoke("start_inference", { imagePaths, threshold, batchSize, operationId })` を呼ぶ。TODO コメントを解消する。未ロードエラーは `renderError` で表示。
- `ops.modelDownload` ハンドラで、`download_model` の直接 await を `spawn_model_download`（`operation_id` 付き）＋進捗購読＋キャンセル配線に置き換える。ダウンロード完了後、保存先を `load_local_model` に渡してセッション保持へ繋ぐ（期待 2.5）。
- サムネイル/プレビューのロードを案 A の `convertFileSrc(path)` 参照へ置き換え、`pngBytesToDataUrl` の使用を撤去する（案 A 採用時）。`placeholder` 分岐は維持する。

## Testing Strategy

### Validation Approach

二段構え。まず未修正コードでバグを再現する反例を出し、次に修正が正しく機能し既存挙動を保持することを検証する。ネットワークダウンロードと実 `ort::Session` 実行は非決定的・環境依存のため、既存方針どおり単体テストではモック（`MockRunner`, `MockDownloader`）で確認し、実起動はスモークに委ねる。

### Exploratory Bug Condition Checking

**Goal**: 修正前に、各不具合が実際に発生することを反例として可視化し、根本原因（結線欠落・状態管理不在・同期実行・JSON 転送）を確認する。反証されれば再仮説を立てる。

**Test Plan**: `generate_handler!` 登録一覧・コマンド定義・State 管理・フロントの `invoke` 有無を検査するテストと、転送方式の観測を書き、未修正コードで失敗させる。

**Test Cases**:
1. **推論起動コマンド不在**: `start_inference` が未定義/未登録であることを確認（未修正では失敗＝コマンドが無い）。
2. **セッション保持不在**: `load_local_model` 呼び出し後に推論から参照できるセッションが存在しないことを確認（未修正では runner を構築できない）。
3. **ダウンロード同期・進捗なし**: `download_model` が同期でブロックし進捗イベントが出ないことを確認（未修正では失敗）。
4. **プレビュー JSON 転送**: `get_preview` の戻り値が `Vec<u8>`（JSON 数値配列化される）でありフロントが `pngBytesToDataUrl` を通ることを確認（未修正では該当）。

**Expected Counterexamples**:
- 推論実行で `inference://progress` が一度も発火しない（起動されていない）。
- ロード後もセッション参照手段が無く推論を組み立てられない。
- ダウンロード中に進捗もキャンセルも効かない。
- 大きい画像でプレビュー転送・パースが重く手動 Base64 が走る。

### Fix Checking

**Goal**: Bug_Condition を満たす全入力に対し、修正版が期待挙動を満たすことを検証する。

**Pseudocode:**
```
FOR ALL input WHERE isBugCondition(input) DO
  result := fixedSystem(input)
  ASSERT expectedBehavior(result)   // Property 1 (a)〜(d)
END FOR
```

具体化:
- 推論: `start_inference` が `spawn_inference_job` を起動し、`RecordingEmitter` 相当で進捗が単調・最終 done==total、完了で `CancelRegistry` が空になることを確認。未ロード時は `AppError`（クラッシュなし）。
- ダウンロード: `spawn_model_download` が即戻りし（ブロックしない）、進捗が通知され、`cancel_operation` で `CancelRegistry` 経由に中断できることをモックで確認。
- プレビュー/サムネイル: 新経路が JSON 数値配列を経ず（案 A ではファイルパス/`convertFileSrc`）に表示可能で、同一入力の再取得でキャッシュが効く（再エンコード回数が減る）ことを確認。

### Preservation Checking

**Goal**: Bug_Condition を満たさない全入力に対し、修正版が元と同じ結果を返すことを検証する。

**Pseudocode:**
```
FOR ALL input WHERE NOT isBugCondition(input) DO
  ASSERT originalSystem(input) = fixedSystem(input)
END FOR
```

**Testing Approach**: プロパティベーステストが有効。多数の入力を自動生成して既存挙動の不変性を広く保証できる。

**Test Plan**: 未修正コードで既存挙動（サービスコマンドの戻り値、同期コアの進捗/キャンセル/部分失敗、`placeholder`、サイズクランプ）を観測し、その挙動を保持することをテストする。

**Test Cases**:
1. **既存コマンド不変**: `list_images`/`bulk_add_tags`/`sort_files`/`capabilities` 等が同じ戻り値・エラー種別を返す（3.1）。
2. **同期コア不変**: `run_inference_job`/`spawn_inference_job` の進捗単調性・キャンセル反映・部分失敗時の処理前状態保持を維持（3.2、既存テストが引き続き通ること）。
3. **DTO 形状不変**: `LoadedModelInfo`/`ModelListing`/`PreviewData`/`ThumbnailData` の既存フィールドが不変（3.4, 3.3, 3.10）。
4. **代替表示・クランプ不変**: 破損画像で `placeholder=true`、サイズが 64〜512 にクランプ、プレビュー縮小規約が維持（3.7, 3.8, 3.9）。
5. **キャンセル戻り値不変**: `cancel_operation` が登録済み `true`/未登録 `false`（3.5）。

### Unit Tests

- `start_inference` の委譲（セッション有→起動、セッション無→`AppError`）をモックセッション/モック runner で検証。
- `spawn_model_download` の即戻り・進捗通知・キャンセル反映をモックダウンローダで検証。
- 新プレビュー/サムネイル経路のパス生成・キャッシュヒット・`placeholder` フォールバックを検証。
- 既存 `file_service`/`model_service`/`adapters` の単体テストが変更後も全て通ること。

### Property-Based Tests

- 既存サービスコマンドの入力を生成し、修正前後で戻り値・エラー種別が一致する（保持）ことを検証。
- サムネイルサイズ生成の入力を生成し、クランプ範囲とアスペクト比保持（拡大なし）の不変性を検証。
- 同期コアの進捗系列が単調非減少・最終 done==total となる不変性を多数入力で検証。

### Integration Tests

- 起動スモーク（`tests/smoke_app.rs` 相当）で `generate_handler!` に新コマンドが登録され、State が `manage` されていることを含めてアプリが起動できることを確認。
- モデル選択 → 推論起動 → 進捗更新 → 完了/キャンセルの一連フローを（実 ort が利用可能な環境で）1〜2 例確認。
- ダウンロード → 保存先ロード → セッション保持 → 推論参照可能の経路を 1 例確認（ネットワークはモックまたは限定実行）。
