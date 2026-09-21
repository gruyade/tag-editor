# Bugfix Requirements Document

## Introduction

タグ推論バッチ機能とプレビュー/サムネイル表示機能について、UI層（frontend/main.js）とRust（Tauri）コマンド層（src-tauri/src/commands/adapters.rs, src-tauri/src/app.rs, src-tauri/src/services/file_service.rs）の結線・実装に起因する以下4つの不具合が発生している。

1. バッチ推論実行ボタンが `operation_id` を発行し進捗イベント購読を開始するのみで、実際にバッチ推論を起動する `invoke` 呼び出しが存在しない。`run_inference_job` / `spawn_inference_job` は実装済みだが、これらを呼び出す `#[tauri::command]` 関数が存在せず、`generate_handler!` にも未登録のため、UIから推論を実行してもRust側の推論処理が一切呼ばれない。
2. モデル選択パネルで選んだ `ModelVariant`（ローカル/リモート）と、実際に `ort::Session` としてロードして保持する経路（アプリ層状態管理）が繋がっていない。リモートモデルはダウンロード後のロード経路も未配線。ロード済みモデルセッションを推論実行時に参照する手段（`tauri::State` 等）がアプリ層に存在しない。
3. `download_model` コマンドが同期的にダウンロードを実行しており、UIスレッド（Tauri非同期コマンドとしてはブロッキング）を長時間占有する可能性がある。設計コメント上は `spawn_download_job` を想定しているが未実装で、UI側も進捗購読なしに直接 `await` している。
4. プレビュー・サムネイル取得（`get_preview`/`get_thumbnail`）が返す `PreviewData.png`/`ThumbnailData.png`（`Vec<u8>`）は、Tauri IPCの戻り値としてJSON数値配列にシリアライズされて転送されている。これによりバイト列サイズが大幅に膨張し、JSON生成・パースのコストが高い。フロントエンドの `pngBytesToDataUrl`（`frontend/main.js`）もバイト配列を手動でBase64化するチャンクループを実行しており、大きい画像ではCPU・メモリ負荷が大きい。加えてRust側にキャッシュがなく、同一画像の再選択でもディスク読込・デコード・リサイズ・PNG再エンコードを毎回やり直している。`tauri.conf.json` の `assetProtocol.enable: true` および `Cargo.toml` の `protocol-asset` featureは有効化済みだが、フロントエンドはこれを使わずJSON経由のバイト列転送に依存しており、表示までの待ち時間が長くなっている。

不具合1〜3は同一の根本原因（アプリ層状態管理と長時間処理spawnの配線がタスク21で未完了）に起因し、不具合4はプレビュー/サムネイル転送方式に起因する別系統の問題だが、いずれもタスク21のUI-Rust結線に関わる不具合として同一spec配下で扱う。既存の `tag-editor` spec の設計方針（長時間処理は別スレッド実行、進捗はイベントpush、キャンセル可能、モデルセッションはserde不可のためアプリ層状態管理）を踏襲する。

## Bug Analysis

### Current Behavior (Defect)

1.1 WHEN ユーザーが推論実行ボタン（`ops.inferRun`）をクリックする THEN システムは `operation_id` を発行し進捗イベント購読とキャンセル待受を開始するが、Rust側の推論処理（`run_inference_job`/`spawn_inference_job`）を一切起動しない
1.2 WHEN 推論実行ボタンがクリックされる THEN システムは進捗イベント（`inference://progress`）が永久に発火しないため、進捗バーが0のまま推論が完了せず、UI上は「進捗イベントを購読中」の表示のまま停止する
1.3 WHEN ユーザーがモデル選択パネルで `ModelVariant`（ローカル/リモート）を選択する THEN システムはそのバリアントを `ort::Session` としてロードし推論で使える状態にする経路を持たない
1.4 WHEN ユーザーがリモートモデルをダウンロードする THEN システムはダウンロード完了後にそのモデルをロードしてセッションとして保持する経路を持たない
1.5 WHEN 推論実行時にロード済みモデルセッションを参照しようとする THEN システムはアプリ層にセッションを保持する状態管理（`tauri::State`等）を持たないため参照できない
1.6 WHEN ユーザーがモデルダウンロードを実行する THEN システムは `download_model` コマンド内でダウンロード処理を同期的に実行し、完了まで応答をブロックする
1.7 WHEN モデルダウンロードが実行される THEN システムはダウンロード進捗をイベントとして通知せず、UIも進捗購読を行わないため、ダウンロード中は完了以外のフィードバックがない
1.8 WHEN ユーザーが画像を選択しプレビューまたはサムネイルを表示する THEN システムは `get_preview`/`get_thumbnail` の戻り値（PNGバイト列）をJSON数値配列としてIPC転送し、画像サイズに比例して転送データ量とパース負荷が膨張した結果、表示までの待ち時間が長くなる
1.9 WHEN フロントエンドが受け取ったPNGバイト列をdata URLへ変換する THEN システムは `pngBytesToDataUrl` によるチャンク単位の `String.fromCharCode` ループで手動Base64化を行い、大きい画像ではCPU・メモリオーバーヘッドが大きくなる
1.10 WHEN 同一画像が再度選択される THEN システムはRust側にプレビュー/サムネイルのキャッシュを持たないため、ディスク読込・デコード・リサイズ・PNG再エンコードを毎回やり直す

### Expected Behavior (Correct)

2.1 WHEN ユーザーが推論実行ボタンをクリックする THEN システムSHALL 選択中のモデル・対象画像・閾値・Batch_Sizeを用いて、`spawn_inference_job` 経由でバッチ推論をバックグラウンドスレッドで起動するTauriコマンドを呼び出す
2.2 WHEN バッチ推論が起動される THEN システムSHALL 発行済みの `operation_id` を用いて進捗イベント（`inference://progress`）を推論の進行に応じて送出し、UIの進捗バーが更新される
2.3 WHEN バッチ推論が完了する THEN システムSHALL 推論結果（成功/失敗/キャンセル件数）をUIに反映し、完了した処理としてキャンセルレジストリから解放する
2.4 WHEN ユーザーがローカルモデルをロードする（`load_local_model`）THEN システムSHALL ロードした `ort::Session` を含む `LoadedModel` をアプリ層の状態管理（`tauri::State`）に保持し、以後の推論実行から参照可能にする
2.5 WHEN ユーザーがリモートモデルをダウンロードして利用する THEN システムSHALL ダウンロード完了後、保存先ディレクトリから同モデルをロードしてアプリ層の状態管理に保持する経路を提供する
2.6 WHEN 推論実行時にモデルが未ロードである THEN システムSHALL 明確なエラー（モデル未選択/未ロード）をUIへ返し、クラッシュせずに処理を中止する
2.7 WHEN ユーザーがモデルダウンロードを実行する THEN システムSHALL ダウンロード処理を別スレッド/タスクでspawnし、UIスレッド（コマンド呼び出し元）をブロックしない
2.8 WHEN モデルダウンロードが進行する THEN システムSHALL 進捗をイベントとして通知し、UISHALL その進捗を購読して表示する
2.9 WHEN ユーザーがモデルダウンロード中にキャンセルを要求する THEN システムSHALL 既存の `cancel_operation`/`CancelRegistry` の仕組みでダウンロードを中断できる
2.10 WHEN ユーザーが画像を選択しプレビューまたはサムネイルを表示する THEN システムSHALL PNGバイト列をJSON数値配列化によるIPC転送コストを排除する方式（asset protocol経由の直接参照、バイナリ転送方式の見直し等、具体手段はdesignフェーズで決定）で転送し、表示までの遅延を許容範囲に収める
2.11 WHEN プレビュー/サムネイルの転送方式が変更される THEN システムSHALL フロントエンド側で `pngBytesToDataUrl` のような手動Base64化のCPU・メモリオーバーヘッドを負わない経路を提供する
2.12 WHEN 同一画像が再度選択される THEN システムSHALL （designフェーズで検討する簡易キャッシュ等により）不要なディスク読込・デコード・リサイズ・PNG再エンコードの繰り返しを削減する

### Unchanged Behavior (Regression Prevention)

3.1 WHEN 既存のFileService/TagService/SortService/RenameService/CaptionService/PlatformService関連コマンド（`list_images`, `get_thumbnail`, `bulk_add_tags`, `sort_files`, `rename_regex`, `find_orphan_captions`, `capabilities`等）が呼び出される THEN システムSHALL CONTINUE TO 既存と同じ挙動・戻り値・エラー種別を返す
3.2 WHEN `run_inference_job` および `spawn_inference_job` の同期コア（Tauri非依存の関数本体）が直接呼び出される THEN システムSHALL CONTINUE TO 既存のテスト（進捗の単調性、キャンセル反映、部分失敗時の状態保持）で検証済みの挙動を維持する
3.3 WHEN `list_models` が呼び出される THEN システムSHALL CONTINUE TO ローカル/リモートの `ModelVariant` 一覧とONNX非対応モデルの除外表示を現状通り返す
3.4 WHEN `load_local_model` がディレクトリパスを受けて呼び出される THEN システムSHALL CONTINUE TO 読込成否とメタ情報（`input_size`, `label_count`）を含む `LoadedModelInfo` をこれまでと同じ形式で返す
3.5 WHEN ユーザーが推論実行中に `cancel_operation` を呼び出す THEN システムSHALL CONTINUE TO `CancelRegistry` を通じて登録済みの処理を中止できる（登録済みなら`true`、未登録なら`false`）
3.6 WHEN 非Windows環境で `capabilities` が呼ばれる THEN システムSHALL CONTINUE TO `windows_only: false` を返しWindows限定UIを非表示にする
3.7 WHEN プレビュー/サムネイル取得に失敗する（破損画像・読込不可等） THEN システムSHALL CONTINUE TO `placeholder=true` の代替表示（サムネイルは「表示不可」、プレビューは「この画像はプレビューできません」）を行い、他項目の表示に波及させない
3.8 WHEN サムネイルサイズが指定される THEN システムSHALL CONTINUE TO [`MIN_THUMBNAIL_SIZE`]=64〜[`MAX_THUMBNAIL_SIZE`]=512の範囲にクランプし、アスペクト比を保って縮小する（拡大はしない）
3.9 WHEN 拡大プレビューが生成される THEN システムSHALL CONTINUE TO [`MAX_PREVIEW_SIZE`]=2048四方を超える場合のみアスペクト比を保って内接縮小し、それ以下は元寸法のまま返す
3.10 WHEN `get_preview`/`get_thumbnail` コマンドが呼び出される THEN システムSHALL CONTINUE TO `PreviewData`/`ThumbnailData` の既存フィールド形状（`width`/`height`/`png`/`placeholder`）を維持する
