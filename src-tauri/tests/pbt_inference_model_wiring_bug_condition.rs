// Feature: inference-model-wiring (bugfix), Property 1: Bug Condition
//
// 探索テスト（バグ条件方法論）。design.md の `isBugCondition` が true を返す
// 4 経路それぞれについて、修正後に成立すべき「期待挙動（結線・状態保持・
// spawn・軽量転送）」を符号化する。未修正コードでは結線が欠落しているため
// これらのアサーションは FAIL し、その失敗が 4 系統のバグの存在を証明する
// 反例となる（design "Exploratory Bug Condition Checking" / Property 1 (a)〜(d)）。
//
// 実装後（タスク 3 系）にこれと同一のテストを再実行し（タスク 3.9）、PASS する
// ことで修正を検証する。
//
// # なぜ「列挙的検査（Scoped PBT）」か
//
// 本 bugfix の不具合は乱数入力で確率的に現れるものではなく、アプリ層の
// 結線が「有る／無い」という決定的な欠落である。したがって proptest による
// ランダム探索ではなく、design の Test Plan（「generate_handler! 登録一覧・
// コマンド定義・State 管理・フロントの invoke 有無を検査する」）に沿って、
// 各バグ経路の具体的な失敗ケースへスコープした列挙的検査を行う。
//
// # 検査手段（コンパイル可能性の担保）
//
// 期待挙動は「まだ存在しない Rust シンボル（start_inference / spawn_model_download /
// ModelSessionState / OwnedOrtRunner / get_thumbnail_path 等）」に依存する。
// （spawn_model_download は local-model-management で spawn_variant_download に改名、
// load_local_model は撤去され start_inference が遅延 DL + load_variant を内包する。）
// これらを直接参照するとテストが **コンパイル不能** になり、クリーンな FAIL
// （読める反例）を出せない。そこで本テストは対象ソース
// （app.rs / adapters.rs / frontend/main.js）と DTO を **ソーステキストとして
// 観測** し、結線の有無を実行時アサーションで判定する。これにより未修正コードでも
// コンパイルは通り、経路ごとに反例メッセージを出して FAIL する。
//
// Validates: Requirements 2.1, 2.2, 2.3, 2.4, 2.5, 2.6, 2.7, 2.8, 2.9, 2.10, 2.11, 2.12

use std::path::{Path, PathBuf};

/// リポジトリルート（`src-tauri` の親）を返す。テストは `src-tauri` を
/// `CARGO_MANIFEST_DIR` として実行されるため、その親が TagEditor ルート。
fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("src-tauri の親（リポジトリルート）が存在するはず")
        .to_path_buf()
}

/// `src-tauri` ディレクトリ（`CARGO_MANIFEST_DIR`）。
fn src_tauri_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// 指定ソースファイルをテキストとして読む。存在しなければ panic（配線対象の
/// ファイルが無いのは環境異常）。
fn read_source(rel_from_repo: &str) -> String {
    let path = repo_root().join(rel_from_repo);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("ソース読込に失敗: {} ({e})", path.display()))
}

/// `src-tauri/src/...` 配下のソースを読む。
fn read_tauri_source(rel_from_src_tauri: &str) -> String {
    let path = src_tauri_dir().join(rel_from_src_tauri);
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("ソース読込に失敗: {} ({e})", path.display()))
}

// ===========================================================================
// 反例 1（推論起動不在）: isBugCondition(input) where input.kind == StartInference
//   AND NOT inferenceCommandInvoked  （design Bug Condition の (a)）
//
// 期待挙動（Property 1 (a)）: バッチ推論を spawn_inference_job 経由で起動する
// #[tauri::command] start_inference が定義され、generate_handler! に登録され、
// フロントの実行ボタンから invoke("start_inference", ...) が呼ばれる。
// ===========================================================================

#[test]
fn counterexample_1_start_inference_command_defined_and_registered() {
    let adapters = read_tauri_source("src/commands/adapters.rs");
    let app = read_tauri_source("src/app.rs");

    // (a-1) start_inference コマンドが adapters.rs に #[tauri::command] として定義されている。
    let defines_command = adapters.contains("pub fn start_inference");
    assert!(
        defines_command,
        "反例1(a): start_inference コマンドが adapters.rs に未定義。\
         推論起動要求(StartInference)を処理する #[tauri::command] が無く、\
         run_inference_job/spawn_inference_job を UI から起動できない（要件 2.1）。"
    );

    // (a-2) spawn_inference_job 経由でバックグラウンド起動している。
    assert!(
        adapters.contains("spawn_inference_job") && adapters.contains("start_inference"),
        "反例1(a): start_inference が spawn_inference_job を起動していない（要件 2.1, 2.2）。"
    );

    // (a-3) generate_handler! に start_inference が登録されている。
    assert!(
        app.contains("start_inference"),
        "反例1(a): app.rs の generate_handler! に start_inference が未登録。\
         UI から invoke できない（要件 2.1）。"
    );
}

#[test]
fn counterexample_1_frontend_invokes_start_inference() {
    let main_js = read_source("frontend/main.js");

    // 推論実行ボタン（ops.inferRun）から実バッチ推論コマンドを invoke している。
    assert!(
        main_js.contains("invoke(\"start_inference\"")
            || main_js.contains("invoke('start_inference'"),
        "反例1(a): frontend/main.js が start_inference を invoke していない。\
         inferRun は operation_id 発行と進捗購読のみで実推論を起動しない\
         （要件 2.1, 2.2、design の TODO 未解消）。"
    );
}

// ===========================================================================
// 反例 2（セッション保持不在）: isBugCondition(input) where
//   input.kind IN [SelectModel, ModelDownloaded]
//   AND NOT loadedSessionRetrievableAtInference  （design Bug Condition の (b)）
//
// 期待挙動（Property 1 (b)）: ロードした LoadedModel（ort::Session を含む）を
// アプリ層 State（ModelSessionState）に保持し、推論から参照できる。
// ===========================================================================

#[test]
fn counterexample_2_model_session_state_managed() {
    let app = read_tauri_source("src/app.rs");
    let adapters = read_tauri_source("src/commands/adapters.rs");

    // (b-1) ロード済みセッションを保持する State 型が存在する。
    let has_session_state =
        adapters.contains("ModelSessionState") || app.contains("ModelSessionState");
    assert!(
        has_session_state,
        "反例2(b): ロード済み LoadedModel を保持する ModelSessionState が存在しない。\
         local-model-management で load_local_model は撤去され、start_inference（遅延 DL +\
         load_variant を内包）と spawn_variant_download が load_variant でロードしたモデルを\
         State へ保持する。その保持先型が無いと推論からセッションを参照できない（要件 2.4, 2.5）。"
    );

    // (b-2) app.rs の Builder で manage されている。
    assert!(
        app.contains(".manage") && app.contains("ModelSessionState"),
        "反例2(b): ModelSessionState が tauri::Builder で manage されていない。\
         アプリ層 State として推論から参照できない（要件 2.4）。"
    );

    // (b-3) load_variant でロードしたモデルを ModelSessionState へ保持する経路を持つ。
    //   local-model-management で load_local_model は撤去され、start_inference が
    //   遅延 DL + load_variant を内包してセッション保持を担い、spawn_variant_download も
    //   完了後 load_variant でロードして同じ ModelSessionState へ格納する。
    //   したがって「load_variant でロードしたモデルを State へ保持する経路」の存在を検査する。
    let load_retains_session = adapters.contains("ModelSessionState")
        && adapters.contains("load_variant")
        && adapters.contains("pub fn start_inference");
    assert!(
        load_retains_session,
        "反例2(b): load_variant でロードしたモデルを ModelSessionState へ保持する経路（start_inference /\
         spawn_variant_download）が無い。ロード済みモデルが推論から参照可能にならない（要件 2.4, 2.5）。"
    );
}

// ===========================================================================
// 反例 3（ダウンロード同期・進捗なし）: isBugCondition(input) where
//   input.kind == DownloadModel
//   AND (downloadRunsSynchronously OR NOT downloadProgressSubscribed)
//   （design Bug Condition の (c)）
//
// 期待挙動（Property 1 (c)）: spawn_variant_download が別スレッドで spawn して
// 即戻り、進捗をイベント通知し、cancel_operation/CancelRegistry で中断できる。
// （local-model-management で spawn_model_download は spawn_variant_download に改名。）
// ===========================================================================

#[test]
fn counterexample_3_spawn_variant_download_defined_and_registered() {
    let adapters = read_tauri_source("src/commands/adapters.rs");
    let app = read_tauri_source("src/app.rs");

    // (c-1) spawn_variant_download コマンドが定義されている。
    assert!(
        adapters.contains("pub fn spawn_variant_download"),
        "反例3(c): spawn_variant_download コマンドが adapters.rs に未定義。\
         別スレッド spawn・進捗通知・キャンセル配線が無い（要件 2.7, 2.8, 2.9）。"
    );

    // (c-2) 別スレッドで spawn し、CancelRegistry と連携している。
    assert!(
        adapters.contains("spawn_variant_download") && adapters.contains("CancelRegistry"),
        "反例3(c): spawn_variant_download がキャンセル配線（CancelRegistry）を持たない（要件 2.9）。"
    );

    // (c-3) generate_handler! に登録されている。
    assert!(
        app.contains("spawn_variant_download"),
        "反例3(c): app.rs の generate_handler! に spawn_variant_download が未登録（要件 2.7）。"
    );
}

#[test]
fn counterexample_3_frontend_uses_spawn_and_progress() {
    let main_js = read_source("frontend/main.js");

    // ダウンロードは spawn_variant_download を進捗購読付きで起動する
    //（同期ダウンロードの直接 await ではない）。
    assert!(
        main_js.contains("invoke(\"spawn_variant_download\"")
            || main_js.contains("invoke('spawn_variant_download'"),
        "反例3(c): frontend/main.js がダウンロードで spawn_variant_download を invoke していない。\
         同期ダウンロードを直接 await しており進捗・キャンセル未配線（要件 2.7, 2.8, 2.9）。"
    );
}

// ===========================================================================
// 反例 4（プレビュー JSON 転送）: isBugCondition(input) where
//   input.kind IN [GetPreview, GetThumbnail] AND transferredAsJsonNumberArray
//   （design Bug Condition の (d)）
//
// 期待挙動（Property 1 (d)）: PNG バイト列を JSON 数値配列化する経路を用いず、
// asset protocol（convertFileSrc）でファイルパス参照し、フロントの手動
// Base64 化（pngBytesToDataUrl）を経ない。
// ===========================================================================

#[test]
fn counterexample_4_lightweight_transfer_path_exists() {
    let adapters = read_tauri_source("src/commands/adapters.rs");

    // (d-1) JSON 数値配列を経ないファイルパス参照経路（案 A）が Rust に追加されている。
    let has_path_route =
        adapters.contains("get_thumbnail_path") || adapters.contains("get_preview_path");
    assert!(
        has_path_route,
        "反例4(d): JSON 数値配列を経ない新経路（get_thumbnail_path/get_preview_path）が未定義。\
         get_preview/get_thumbnail の png(Vec<u8>) が JSON 数値配列で転送される（要件 2.10, 2.12）。"
    );
}

#[test]
fn counterexample_4_frontend_drops_manual_base64() {
    let main_js = read_source("frontend/main.js");

    // (d-2) フロントは convertFileSrc（asset protocol）でパス参照する。
    assert!(
        main_js.contains("convertFileSrc"),
        "反例4(d): frontend/main.js が convertFileSrc（asset protocol）を使っていない。\
         PNG を JSON 数値配列で受け取っている（要件 2.10, 2.11）。"
    );

    // (d-3) 手動 Base64 化（pngBytesToDataUrl）の使用を撤去している。
    //   定義が残っていても、サムネイル/プレビュー表示から呼ばれていないこと。
    let calls_manual_base64 = main_js.contains("pngBytesToDataUrl(thumb.png")
        || main_js.contains("pngBytesToDataUrl(preview.png");
    assert!(
        !calls_manual_base64,
        "反例4(d): frontend/main.js が pngBytesToDataUrl による手動 Base64 化を\
         まだ使用している。asset protocol 経由の軽量転送に置き換わっていない（要件 2.11）。"
    );
}
