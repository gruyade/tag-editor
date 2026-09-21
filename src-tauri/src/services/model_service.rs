//! ModelService（タスク 17.1）: モデル一覧と ONNX フィルタ。
//!
//! 本モジュールは推論モデルの「一覧化」を担う。組み込みのリモート候補
//! （WD14 系の各バリアント・ML-Danbooru 系）と、`Local_Model_Dir` 内で検出した
//! ローカルモデルを統合し、ONNX 形式を持つ候補のみを利用可能な一覧として返す。
//!
//! 要件との対応:
//!
//! - 要件 15.1: 利用可能な Model_Variant（WD14 系・ML-Danbooru 系・ローカル検出）
//!   の一覧を提示する。→ [`list_models`] が `available` に集約する。
//! - 要件 15.5: TensorFlow 専用形式（`.onnx` を持たない DeepDanbooru プロジェクト
//!   等）は選択対象に含めず、対象外である旨を提示する。→ `onnx_available == false`
//!   の候補は `available` から除外し、`excluded` へ振り分けて UI が「対象外」表示
//!   に使えるようにする。
//!
//! # 設計上の取り決め
//!
//! - フィルタの基準は各候補の [`ModelVariant::onnx_available`] フラグ **のみ**。
//!   これにより Property 24（一覧 == `onnx_available == true` の集合）が候補集合の
//!   組み立て方に依らず成り立つ。
//! - リモート候補は呼び出し側から `remote_candidates` として受け取る形にして
//!   テスト可能性を確保する。既知の WD14／ML-Danbooru バリアントは
//!   [`builtin_remote_variants`] が提供し、実運用ではこれを渡す想定。
//! - ローカルモデルの検出は「`.onnx` ファイルとタグ定義ファイル（`.csv` または
//!   `.json`）の対」を要件 15.2／用語定義（`Local_Model_Dir`）に従って判定する。
//!   タグ定義を欠く `.onnx` 単体は不完全なため検出対象にしない。
//!
//! _Requirements: 15.1, 15.5_

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::models::{ModelFamily, ModelLocation, ModelVariant};

/// モデル一覧の結果。
///
/// `available` は UI の選択肢として提示する候補（`onnx_available == true`）。
/// `excluded` は TensorFlow 専用等で `.onnx` を持たない候補（要件 15.5 の
/// 「対象外」提示に用いる）。両者の和が入力候補全体（重複排除後）に一致する。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelListing {
    /// 選択可能な Model_Variant（`onnx_available == true`）。
    pub available: Vec<ModelVariant>,
    /// 対象外の Model_Variant（`.onnx` を持たない、要件 15.5）。
    pub excluded: Vec<ModelVariant>,
}

/// 既知の WD14 系／ML-Danbooru 系リモートバリアントを返す。
///
/// いずれも ONNX 形式を提供するため `onnx_available = true`。実運用では本関数の
/// 戻り値を [`list_models`] の `remote_candidates` に渡す。`location` は
/// [`ModelLocation::Remote`]（HuggingFace リポジトリ）で表現する。
///
/// WD14 系は用語定義（Model_Variant）に挙がる ViT / ConvNeXT / ConvNeXTV2 /
/// SwinV2 / MoaT の 5 バリアントに、ML-Danbooru 系 1 件を加えた計 6 件。
pub fn builtin_remote_variants() -> Vec<ModelVariant> {
    let wd14 = [
        (
            "wd14-vit",
            "WD14 ViT",
            "SmilingWolf/wd-vit-tagger-v3",
        ),
        (
            "wd14-convnext",
            "WD14 ConvNeXT",
            "SmilingWolf/wd-convnext-tagger-v3",
        ),
        (
            "wd14-convnextv2",
            "WD14 ConvNeXTV2",
            "SmilingWolf/wd-v1-4-convnextv2-tagger-v2",
        ),
        (
            "wd14-swinv2",
            "WD14 SwinV2",
            "SmilingWolf/wd-swinv2-tagger-v3",
        ),
        (
            "wd14-moat",
            "WD14 MoaT",
            "SmilingWolf/wd-v1-4-moat-tagger-v2",
        ),
    ];

    let mut variants: Vec<ModelVariant> = wd14
        .into_iter()
        .map(|(id, display, repo)| ModelVariant {
            id: id.to_string(),
            display_name: display.to_string(),
            family: ModelFamily::Wd14,
            location: ModelLocation::Remote(repo.to_string()),
            onnx_available: true,
        })
        .collect();

    variants.push(ModelVariant {
        id: "ml-danbooru".to_string(),
        display_name: "ML-Danbooru".to_string(),
        family: ModelFamily::MlDanbooru,
        location: ModelLocation::Remote("deepghs/ml-danbooru-onnx".to_string()),
        onnx_available: true,
    });

    variants
}

/// `Local_Model_Dir` を走査し、検出したローカルモデルを [`ModelVariant`] 化する。
///
/// ローカルモデルは「`.onnx` ファイルと、タグ定義ファイル（`.csv` または
/// `.json`）の対」で構成される（用語定義 `Local_Model_Dir`、要件 15.2）。本関数は
/// 次の 2 通りのレイアウトを検出する:
///
/// 1. `dir` 直下に `.onnx` と `.csv`/`.json` が同居するフラット構成。
/// 2. `dir` 直下の各サブディレクトリ内に `.onnx` と `.csv`/`.json` が対で存在する
///    構成（1 モデル 1 ディレクトリ）。
///
/// `.onnx` はあるがタグ定義を欠くエントリは不完全とみなし、検出対象にしない
/// （読み込めないモデルを一覧に載せないため）。検出した各モデルは
/// `family = Local` / `location = Local(パス)` / `onnx_available = true` とする。
/// `id` はディレクトリ名（フラット構成では `.onnx` のファイルステム）を用いる。
///
/// 走査に失敗した場合（存在しない・権限不足等）は空ベクトルを返す（一覧化は
/// リモート候補のみで継続できるため、ここではエラーにしない）。
fn scan_local_models(dir: &Path) -> Vec<ModelVariant> {
    let mut found = Vec::new();

    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return found,
    };

    // まず dir 直下自体がフラット構成のモデルかを判定する。
    if let Some(onnx) = onnx_with_tagdef(dir) {
        let id = dir
            .file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| onnx_stem(&onnx));
        found.push(local_variant(id, &onnx));
    }

    // 次にサブディレクトリを 1 モデル 1 ディレクトリ構成として走査する。
    let mut subdirs: Vec<std::path::PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    // 決定的な順序で返すためソートする。
    subdirs.sort();

    for sub in subdirs {
        if let Some(onnx) = onnx_with_tagdef(&sub) {
            let id = sub
                .file_name()
                .and_then(|s| s.to_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| onnx_stem(&onnx));
            found.push(local_variant(id, &onnx));
        }
    }

    found
}

/// 指定ディレクトリ直下に `.onnx` とタグ定義（`.csv`/`.json`）が揃っていれば、
/// その `.onnx` のパスを返す。揃っていなければ `None`。
fn onnx_with_tagdef(dir: &Path) -> Option<std::path::PathBuf> {
    let read = std::fs::read_dir(dir).ok()?;
    let mut onnx: Option<std::path::PathBuf> = None;
    let mut has_tagdef = false;

    for entry in read.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
        {
            Some(ext) if ext == "onnx" => {
                if onnx.is_none() {
                    onnx = Some(path);
                }
            }
            Some(ext) if ext == "csv" || ext == "json" => has_tagdef = true,
            _ => {}
        }
    }

    match (onnx, has_tagdef) {
        (Some(path), true) => Some(path),
        _ => None,
    }
}

/// `.onnx` パスからファイルステム（拡張子なしの名前）を取り出す。
fn onnx_stem(onnx: &Path) -> String {
    onnx.file_stem()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "local-model".to_string())
}

/// ローカル検出モデルの [`ModelVariant`] を組み立てる。
fn local_variant(id: String, onnx: &Path) -> ModelVariant {
    ModelVariant {
        display_name: id.clone(),
        id,
        family: ModelFamily::Local,
        location: ModelLocation::Local(onnx.to_string_lossy().into_owned()),
        onnx_available: true,
    }
}

/// リモート候補とローカル検出モデルを統合し、ONNX フィルタを適用して一覧化する。
///
/// 手順:
///
/// 1. `remote_candidates`（実運用では [`builtin_remote_variants`] の戻り値）と、
///    `local_model_dir` が `Some` の場合は [`scan_local_models`] の検出結果を
///    連結して全候補集合を作る。
/// 2. 各候補を [`ModelVariant::onnx_available`] で振り分ける。`true` は
///    `available`、`false` は `excluded`（要件 15.5 の「対象外」提示用）。
///
/// これにより `available` は「`onnx_available == true` の候補集合」とちょうど
/// 一致する（Property 24）。ローカル検出モデルは常に `onnx_available = true` の
/// ため必ず `available` に入る。
///
/// # 引数
///
/// - `local_model_dir`: ローカルモデル配置ディレクトリ。`None` ならローカル走査を
///   行わずリモート候補のみで一覧化する。
/// - `remote_candidates`: リモートの候補集合。TF 専用（`onnx_available == false`）を
///   含めてよい。含めた場合は `excluded` へ振り分けられる。
pub fn list_models(
    local_model_dir: Option<&Path>,
    remote_candidates: &[ModelVariant],
) -> ModelListing {
    let mut all: Vec<ModelVariant> = remote_candidates.to_vec();

    if let Some(dir) = local_model_dir {
        all.extend(scan_local_models(dir));
    }

    let mut listing = ModelListing::default();
    for variant in all {
        if variant.onnx_available {
            listing.available.push(variant);
        } else {
            listing.excluded.push(variant);
        }
    }
    listing
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    /// テスト用のリモート候補を組み立てる。
    fn remote(id: &str, onnx_available: bool) -> ModelVariant {
        ModelVariant {
            id: id.to_string(),
            display_name: id.to_string(),
            family: ModelFamily::Wd14,
            location: ModelLocation::Remote(format!("repo/{id}")),
            onnx_available,
        }
    }

    #[test]
    fn builtin_variants_cover_wd14_and_ml_danbooru_all_onnx() {
        let variants = builtin_remote_variants();
        // WD14 5 バリアント + ML-Danbooru 1 = 6 件。
        assert_eq!(variants.len(), 6);
        // 全て ONNX 形式を持つ。
        assert!(variants.iter().all(|v| v.onnx_available));

        let wd14 = variants
            .iter()
            .filter(|v| v.family == ModelFamily::Wd14)
            .count();
        let mld = variants
            .iter()
            .filter(|v| v.family == ModelFamily::MlDanbooru)
            .count();
        assert_eq!(wd14, 5);
        assert_eq!(mld, 1);
    }

    #[test]
    fn tf_only_candidate_is_excluded_from_available() {
        let candidates = vec![
            remote("wd14-vit", true),
            remote("deepdanbooru", false), // TF 専用（.onnx なし）
        ];
        let listing = list_models(None, &candidates);

        let available_ids: Vec<&str> =
            listing.available.iter().map(|v| v.id.as_str()).collect();
        let excluded_ids: Vec<&str> =
            listing.excluded.iter().map(|v| v.id.as_str()).collect();

        assert_eq!(available_ids, vec!["wd14-vit"]);
        assert_eq!(excluded_ids, vec!["deepdanbooru"]);
        // available は onnx_available==true の集合とちょうど一致（Property 24 の骨子）。
        assert!(listing.available.iter().all(|v| v.onnx_available));
    }

    #[test]
    fn local_dir_with_onnx_and_csv_is_detected() {
        let dir = tempdir().unwrap();
        let model = dir.path().join("my-model");
        fs::create_dir(&model).unwrap();
        fs::write(model.join("model.onnx"), b"onnx").unwrap();
        fs::write(model.join("selected_tags.csv"), b"tag,category").unwrap();

        let listing = list_models(Some(dir.path()), &[]);

        assert_eq!(listing.available.len(), 1);
        let v = &listing.available[0];
        assert_eq!(v.id, "my-model");
        assert_eq!(v.family, ModelFamily::Local);
        assert!(v.onnx_available);
        match &v.location {
            ModelLocation::Local(p) => assert!(p.ends_with("model.onnx")),
            other => panic!("ローカルモデルは Local パスを持つべき: {other:?}"),
        }
        assert!(listing.excluded.is_empty());
    }

    #[test]
    fn local_dir_with_onnx_and_json_is_detected() {
        let dir = tempdir().unwrap();
        let model = dir.path().join("json-model");
        fs::create_dir(&model).unwrap();
        fs::write(model.join("model.onnx"), b"onnx").unwrap();
        fs::write(model.join("tags.json"), b"[]").unwrap();

        let listing = list_models(Some(dir.path()), &[]);
        assert_eq!(listing.available.len(), 1);
        assert_eq!(listing.available[0].id, "json-model");
    }

    #[test]
    fn onnx_without_tagdef_is_not_detected() {
        let dir = tempdir().unwrap();
        let model = dir.path().join("incomplete");
        fs::create_dir(&model).unwrap();
        // タグ定義ファイルが無い .onnx 単体は検出しない。
        fs::write(model.join("model.onnx"), b"onnx").unwrap();

        let listing = list_models(Some(dir.path()), &[]);
        assert!(listing.available.is_empty());
        assert!(listing.excluded.is_empty());
    }

    #[test]
    fn flat_local_dir_layout_is_detected() {
        // dir 直下に .onnx と .csv が同居するフラット構成。
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("wd.onnx"), b"onnx").unwrap();
        fs::write(dir.path().join("wd.csv"), b"tag,category").unwrap();

        let listing = list_models(Some(dir.path()), &[]);
        assert_eq!(listing.available.len(), 1);
        assert_eq!(listing.available[0].family, ModelFamily::Local);
    }

    #[test]
    fn mixes_remote_and_local_and_applies_filter() {
        let dir = tempdir().unwrap();
        let model = dir.path().join("local-a");
        fs::create_dir(&model).unwrap();
        fs::write(model.join("m.onnx"), b"onnx").unwrap();
        fs::write(model.join("m.csv"), b"tag").unwrap();

        let candidates = vec![
            remote("wd14-vit", true),
            remote("tf-only", false),
        ];
        let listing = list_models(Some(dir.path()), &candidates);

        // available = リモートの ONNX 1 件 + ローカル 1 件。
        let mut available_ids: Vec<String> =
            listing.available.iter().map(|v| v.id.clone()).collect();
        available_ids.sort();
        assert_eq!(available_ids, vec!["local-a", "wd14-vit"]);

        // excluded = TF 専用 1 件。
        assert_eq!(listing.excluded.len(), 1);
        assert_eq!(listing.excluded[0].id, "tf-only");

        // Property 24 の骨子: available はちょうど onnx_available==true の集合。
        assert!(listing.available.iter().all(|v| v.onnx_available));
        assert!(listing.excluded.iter().all(|v| !v.onnx_available));
    }

    #[test]
    fn missing_local_dir_yields_no_local_models() {
        let dir = tempdir().unwrap();
        let missing = dir.path().join("does-not-exist");
        let listing = list_models(Some(&missing), &[remote("wd14-vit", true)]);
        // 走査失敗はエラーにせず、リモート候補のみで一覧化する。
        assert_eq!(listing.available.len(), 1);
        assert_eq!(listing.available[0].id, "wd14-vit");
    }
}

// ---------------------------------------------------------------------------
// タスク 17.3: ローカルモデル読込
// ---------------------------------------------------------------------------
//
// 本節は「`.onnx` とタグ定義（`.csv`/`.json`）の対」を実際に読み込み、推論に使う
// [`LoadedModel`]（ONNX セッション + ラベル定義 + 入力規約）を構築する。
//
// 要件との対応:
//
// - 要件 15.2: ローカルの `.onnx` とタグ定義の対を読み込んでモデルを利用可能に
//   する。→ [`load_local_model`] が `.onnx` の検出・タグ定義の解析・ONNX セッション
//   構築を行い [`LoadedModel`] を返す。
// - 要件 15.6: ONNX の読込に失敗、またはタグ定義が欠落している場合はエラーとし、
//   推論を無効のまま維持する（部分的に壊れたモデルを「読み込めた」ことにしない）。
//   → 検出・解析・セッション構築のいずれかが失敗すると `Err(ModelLoad)` を返し、
//   呼び出し側は `LoadedModel` を得られない = 推論は無効のまま。
//
// # 設計上の取り決め（テスト可能性のための分割）
//
// 実 `ort::Session` の構築は共有ライブラリ（`load-dynamic`）を要するため、単体
// テストで常に成功させられない。そこで処理を次の 2 段に分離する:
//
// 1. タグ定義の解析・検証と `.onnx` パスの特定 …… ランタイム非依存で完全にテスト可能。
//    - [`parse_tag_definition`]: `.csv`/`.json` を [`LabelDef`] 列へ解析する。
//    - [`onnx_with_tagdef`]（既存, タスク 17.1）: 対の検出に再利用する。
// 2. ONNX セッションの構築 …… 実ランタイムを要する（[`load_local_model`] の後段）。
//
// これにより「タグ定義欠落」「タグ定義不正」「`.onnx` 欠落」といった 15.6 の異常系は
// ランタイム無しでも検証できる。実セッション構築を伴う正常系のテストはランタイムが
// 無い環境ではスキップする。
//
// # 入力規約の既定値
//
// WD14 系の一般的な規約に合わせ、`input_size = 448`・`channel_order = Bgr` を既定と
// する（InferenceService の前処理規約と整合）。モデルメタから解決可能になった場合は
// 後続タスクで上書きしてよい。
//
// _Requirements: 15.2, 15.6_

use crate::error::{AppError, AppResult};
use crate::models::{ChannelOrder, LabelDef, LoadedModel, TagCategory};

/// ローカルモデル読込の既定入力サイズ（WD14 系: 448）。
const DEFAULT_INPUT_SIZE: u32 = 448;

/// ローカルモデル読込の既定チャンネル順（WD14 系: BGR）。
const DEFAULT_CHANNEL_ORDER: ChannelOrder = ChannelOrder::Bgr;

/// タグ定義ファイル（`.csv` / `.json`）を [`LabelDef`] 列へ解析する。
///
/// ランタイム非依存の純粋な解析で、[`load_local_model`] のテスト可能な中核。
///
/// # 対応フォーマット
///
/// - **CSV**: WD14 の `selected_tags.csv` 相当。ヘッダ行に `name` 列（あれば
///   `category` 列も）を含む形式を主対象とする。ヘッダから列位置を解決し、以降の
///   各行から `name` と `category` を取り出す。ヘッダに `name` 列が無い場合は
///   「先頭列 = name、（あれば）2 列目 = category」の位置ベースで解釈する。
///   `category` は WD14 の数値コード（`9`=rating, `4`=character, その他=general）
///   または文字列（`rating`/`character`/`general`、大小無視）を受理する。
/// - **JSON**: 次のいずれか。
///   - 文字列配列 `["tag_a", "tag_b", ...]`（全て `General` 扱い）。
///   - オブジェクト配列 `[{"name": "...", "category": ...}, ...]`
///     （`category` は数値コードまたは文字列。省略時は `General`）。
///
/// # エラー（要件 15.6）
///
/// - ファイルを開けない/読めない → `Err(ModelLoad)`。
/// - 解析に失敗する（不正な JSON、CSV に有効行が無い等） → `Err(ModelLoad)`。
/// - 解析はできたが有効なラベルが 0 件 → `Err(ModelLoad)`。
///
/// いずれの場合も呼び出し側は `LoadedModel` を得られず、推論は無効のまま維持される。
pub fn parse_tag_definition(path: &Path) -> AppResult<Vec<LabelDef>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase());

    let content = std::fs::read_to_string(path).map_err(|e| {
        AppError::model_load(format!("タグ定義を読み込めません: {e}"))
            .with_path(path.to_string_lossy().into_owned())
    })?;

    let labels = match ext.as_deref() {
        Some("json") => parse_tag_definition_json(&content),
        Some("csv") => parse_tag_definition_csv(&content),
        // 拡張子不明でも内容から JSON/CSV を推測せず、CSV として解釈を試みる。
        _ => parse_tag_definition_csv(&content),
    }
    .map_err(|msg| {
        AppError::model_load(msg).with_path(path.to_string_lossy().into_owned())
    })?;

    if labels.is_empty() {
        return Err(AppError::model_load("タグ定義に有効なラベルがありません")
            .with_path(path.to_string_lossy().into_owned()));
    }

    Ok(labels)
}

/// WD14 数値コード（または文字列）を [`TagCategory`] へ写像する。
///
/// 数値: `9` = rating, `4` = character, それ以外（`0` 等） = general。
/// 文字列: `rating` / `character` は各カテゴリ、それ以外は general（大小無視）。
fn category_from_token(token: &str) -> TagCategory {
    let t = token.trim();
    if let Ok(code) = t.parse::<i64>() {
        return match code {
            9 => TagCategory::Rating,
            4 => TagCategory::Character,
            _ => TagCategory::General,
        };
    }
    match t.to_ascii_lowercase().as_str() {
        "rating" => TagCategory::Rating,
        "character" => TagCategory::Character,
        _ => TagCategory::General,
    }
}

/// CSV テキストを [`LabelDef`] 列へ解析する。失敗時は説明メッセージを返す。
///
/// 単純な行/カンマ分割（引用符・エスケープ非対応）で WD14 `selected_tags.csv` の
/// 標準形（`tag_id,name,category`）を主対象とする。ヘッダから `name`/`category`
/// 列位置を解決し、`name` 列が見つからなければ位置ベース（先頭=name, 2列目=category）
/// で解釈する。
fn parse_tag_definition_csv(content: &str) -> Result<Vec<LabelDef>, String> {
    let mut lines = content
        .lines()
        .map(|l| l.trim_end_matches('\r'))
        .filter(|l| !l.trim().is_empty());

    let header = match lines.next() {
        Some(h) => h,
        None => return Err("CSV が空です".to_string()),
    };

    let header_cols: Vec<String> = header
        .split(',')
        .map(|c| c.trim().to_ascii_lowercase())
        .collect();

    let name_idx = header_cols.iter().position(|c| c == "name");
    let category_idx = header_cols.iter().position(|c| c == "category");

    // ヘッダに name 列が無い場合は位置ベース解釈にフォールバックする。この場合、
    // 先頭行（header）自体もデータ行として扱う必要があるため、行イテレータを
    // 作り直す。
    let (name_col, category_col, use_header_as_data) = match name_idx {
        Some(n) => (n, category_idx, false),
        None => (0usize, Some(1usize), true),
    };

    let mut labels = Vec::new();

    // 位置ベースの場合はヘッダ行もデータとして処理する。
    let data_lines: Box<dyn Iterator<Item = &str>> = if use_header_as_data {
        Box::new(std::iter::once(header).chain(lines))
    } else {
        Box::new(lines)
    };

    for line in data_lines {
        let cols: Vec<&str> = line.split(',').collect();
        let name = match cols.get(name_col) {
            Some(v) => v.trim(),
            None => continue,
        };
        if name.is_empty() {
            continue;
        }
        let category = category_col
            .and_then(|ci| cols.get(ci))
            .map(|c| category_from_token(c))
            .unwrap_or(TagCategory::General);
        labels.push(LabelDef {
            name: name.to_string(),
            category,
        });
    }

    Ok(labels)
}

/// JSON テキストを [`LabelDef`] 列へ解析する。失敗時は説明メッセージを返す。
///
/// 文字列配列（全て general）とオブジェクト配列（`name` 必須, `category` 任意）の
/// いずれも受理する。混在も許容する。
fn parse_tag_definition_json(content: &str) -> Result<Vec<LabelDef>, String> {
    let value: serde_json::Value = serde_json::from_str(content)
        .map_err(|e| format!("JSON を解析できません: {e}"))?;

    let array = value
        .as_array()
        .ok_or_else(|| "JSON タグ定義は配列である必要があります".to_string())?;

    let mut labels = Vec::new();
    for item in array {
        match item {
            serde_json::Value::String(name) => {
                let name = name.trim();
                if !name.is_empty() {
                    labels.push(LabelDef {
                        name: name.to_string(),
                        category: TagCategory::General,
                    });
                }
            }
            serde_json::Value::Object(map) => {
                let name = map
                    .get("name")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim())
                    .unwrap_or("");
                if name.is_empty() {
                    continue;
                }
                let category = match map.get("category") {
                    Some(serde_json::Value::String(s)) => category_from_token(s),
                    Some(serde_json::Value::Number(n)) => {
                        category_from_token(&n.to_string())
                    }
                    _ => TagCategory::General,
                };
                labels.push(LabelDef {
                    name: name.to_string(),
                    category,
                });
            }
            _ => {}
        }
    }

    Ok(labels)
}

/// ローカルモデルディレクトリから [`LoadedModel`] を構築する（要件 15.2）。
///
/// `dir` 直下に `.onnx` とタグ定義（`.csv`/`.json`）が対で存在することを
/// [`onnx_with_tagdef`]（タスク 17.1 の検出ロジック）で確認し、タグ定義を
/// [`parse_tag_definition`] で解析したうえで ONNX セッションを構築する。
///
/// # エラー（要件 15.6: いずれも推論を無効のまま維持）
///
/// - `.onnx` またはタグ定義が欠落 → `Err(ModelLoad)`。
/// - タグ定義が空/不正 → `Err(ModelLoad)`（[`parse_tag_definition`] 由来）。
/// - ONNX セッションの構築失敗（形式不正・ランタイム不在等） → `Err(ModelLoad)`。
///
/// # 入力規約
///
/// `input_size`/`channel_order` は WD14 既定（448 / BGR）を用いる。モデルメタから
/// 解決可能になった場合は後続タスクで精緻化する。
pub fn load_local_model(dir: &Path) -> AppResult<LoadedModel> {
    // 1) `.onnx` とタグ定義の対を検出する（対が無ければ 15.6 のエラー）。
    let onnx_path = onnx_with_tagdef(dir).ok_or_else(|| {
        AppError::model_load(
            "モデルディレクトリに .onnx とタグ定義（.csv/.json）の対がありません",
        )
        .with_path(dir.to_string_lossy().into_owned())
    })?;

    // 2) タグ定義ファイルを特定して解析する（欠落/不正は 15.6 のエラー）。
    let tagdef_path = find_tag_definition(dir).ok_or_else(|| {
        AppError::model_load("タグ定義ファイル（.csv/.json）が見つかりません")
            .with_path(dir.to_string_lossy().into_owned())
    })?;
    let labels = parse_tag_definition(&tagdef_path)?;

    // 3) ONNX セッションを構築する（構築失敗は 15.6 のエラー）。
    let session = build_session(&onnx_path)?;

    Ok(LoadedModel {
        session,
        input_size: DEFAULT_INPUT_SIZE,
        channel_order: DEFAULT_CHANNEL_ORDER,
        labels,
    })
}

/// ディレクトリ直下の最初のタグ定義ファイル（`.csv`/`.json`）のパスを返す。
///
/// [`onnx_with_tagdef`] が対の有無のみを判定するのに対し、こちらは解析対象の
/// 具体パスを取り出す。`.csv` を `.json` より優先する。
fn find_tag_definition(dir: &Path) -> Option<std::path::PathBuf> {
    let read = std::fs::read_dir(dir).ok()?;
    let mut csv: Option<std::path::PathBuf> = None;
    let mut json: Option<std::path::PathBuf> = None;

    for entry in read.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        match path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
        {
            Some(ext) if ext == "csv" && csv.is_none() => csv = Some(path),
            Some(ext) if ext == "json" && json.is_none() => json = Some(path),
            _ => {}
        }
    }

    csv.or(json)
}

/// `.onnx` ファイルから ONNX 推論セッションを構築する。
///
/// 実 `ort::Session` の構築で、共有ライブラリ（`load-dynamic`）を要する。構築に
/// 失敗した場合は要件 15.6 に従い `Err(ModelLoad)` を返す（推論は無効のまま）。
fn build_session(onnx_path: &Path) -> AppResult<ort::session::Session> {
    ort::session::Session::builder()
        .and_then(|mut b| b.commit_from_file(onnx_path))
        .map_err(|e| {
            AppError::model_load(format!("ONNX セッションを構築できません: {e}"))
                .with_path(onnx_path.to_string_lossy().into_owned())
        })
}

#[cfg(test)]
mod load_local_model_tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    #[test]
    fn parses_wd14_style_csv_with_categories() {
        // WD14 selected_tags.csv 相当: tag_id,name,category（9=rating,0=general,4=character）。
        let dir = tempdir().unwrap();
        let csv = dir.path().join("selected_tags.csv");
        fs::write(
            &csv,
            "tag_id,name,category\n\
             9,general,9\n\
             1,solo,0\n\
             2,hatsune_miku,4\n",
        )
        .unwrap();

        let labels = parse_tag_definition(&csv).unwrap();
        assert_eq!(labels.len(), 3);
        assert_eq!(labels[0].name, "general");
        assert_eq!(labels[0].category, TagCategory::Rating);
        assert_eq!(labels[1].name, "solo");
        assert_eq!(labels[1].category, TagCategory::General);
        assert_eq!(labels[2].name, "hatsune_miku");
        assert_eq!(labels[2].category, TagCategory::Character);
    }

    #[test]
    fn parses_csv_without_name_header_positionally() {
        // ヘッダに name 列が無い場合は位置ベース（先頭=name, 2列目=category）で解釈し、
        // 先頭行もデータとして扱う。
        let dir = tempdir().unwrap();
        let csv = dir.path().join("tags.csv");
        fs::write(&csv, "solo,0\n1girl,0\ncharacter_x,4\n").unwrap();

        let labels = parse_tag_definition(&csv).unwrap();
        assert_eq!(labels.len(), 3);
        assert_eq!(labels[0].name, "solo");
        assert_eq!(labels[0].category, TagCategory::General);
        assert_eq!(labels[2].name, "character_x");
        assert_eq!(labels[2].category, TagCategory::Character);
    }

    #[test]
    fn parses_json_string_array_as_general() {
        let dir = tempdir().unwrap();
        let json = dir.path().join("tags.json");
        fs::write(&json, r#"["solo", "1girl", "smile"]"#).unwrap();

        let labels = parse_tag_definition(&json).unwrap();
        assert_eq!(labels.len(), 3);
        assert!(labels.iter().all(|l| l.category == TagCategory::General));
        assert_eq!(labels[0].name, "solo");
        assert_eq!(labels[2].name, "smile");
    }

    #[test]
    fn parses_json_object_array_with_categories() {
        let dir = tempdir().unwrap();
        let json = dir.path().join("tags.json");
        fs::write(
            &json,
            r#"[
                {"name": "general", "category": "rating"},
                {"name": "solo", "category": 0},
                {"name": "miku", "category": 4}
            ]"#,
        )
        .unwrap();

        let labels = parse_tag_definition(&json).unwrap();
        assert_eq!(labels.len(), 3);
        assert_eq!(labels[0].category, TagCategory::Rating);
        assert_eq!(labels[1].category, TagCategory::General);
        assert_eq!(labels[2].category, TagCategory::Character);
    }

    #[test]
    fn empty_tag_definition_is_model_load_error() {
        // ヘッダのみで有効なデータ行が無い CSV は「有効ラベル 0 件」でエラー。
        let dir = tempdir().unwrap();
        let csv = dir.path().join("empty.csv");
        fs::write(&csv, "tag_id,name,category\n").unwrap();

        let err = parse_tag_definition(&csv).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::ModelLoad);
    }

    #[test]
    fn malformed_json_tag_definition_is_model_load_error() {
        let dir = tempdir().unwrap();
        let json = dir.path().join("bad.json");
        fs::write(&json, "{ not valid json ").unwrap();

        let err = parse_tag_definition(&json).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::ModelLoad);
    }

    #[test]
    fn json_non_array_tag_definition_is_model_load_error() {
        let dir = tempdir().unwrap();
        let json = dir.path().join("obj.json");
        fs::write(&json, r#"{"name": "solo"}"#).unwrap();

        let err = parse_tag_definition(&json).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::ModelLoad);
    }

    #[test]
    fn missing_tag_definition_yields_model_load_error() {
        // .onnx はあるがタグ定義が無い → 15.6 のエラー（推論無効のまま維持）。
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("model.onnx"), b"onnx").unwrap();

        let err = load_local_model(dir.path()).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::ModelLoad);
    }

    #[test]
    fn missing_onnx_yields_model_load_error() {
        // タグ定義はあるが .onnx が無い → 15.6 のエラー。
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("selected_tags.csv"), b"tag_id,name,category\n1,solo,0\n")
            .unwrap();

        let err = load_local_model(dir.path()).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::ModelLoad);
    }

    #[test]
    fn empty_dir_yields_model_load_error() {
        let dir = tempdir().unwrap();
        let err = load_local_model(dir.path()).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::ModelLoad);
    }

    #[test]
    fn invalid_onnx_bytes_yield_model_load_error_when_runtime_available() {
        // .onnx とタグ定義の対は揃うが、.onnx の中身が不正。
        // ONNX ランタイム（load-dynamic の共有ライブラリ）が利用可能な環境では
        // セッション構築が失敗し ModelLoad になる。ランタイム不在の環境でも
        // build_session は Err を返すため、いずれにせよ Err(ModelLoad) を期待する。
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("model.onnx"), b"not a real onnx model").unwrap();
        fs::write(
            dir.path().join("selected_tags.csv"),
            b"tag_id,name,category\n1,solo,0\n",
        )
        .unwrap();

        let result = load_local_model(dir.path());
        // タグ定義は妥当なので、失敗するとすればセッション構築段階（15.6）。
        let err = result.expect_err("不正な .onnx はモデル読込エラーになるべき");
        assert_eq!(err.kind, crate::error::AppErrorKind::ModelLoad);
    }
}

// ---------------------------------------------------------------------------
// タスク 17.4: リモートモデルのダウンロード・保存
// ---------------------------------------------------------------------------
//
// 本節はリモートの [`ModelVariant`]（`ModelLocation::Remote(repo)`）から
// `model.onnx` とタグ定義ファイルを取得し、`dest_dir` へローカル保存する。保存後は
// タスク 17.3 の [`load_local_model`] が `dest_dir` を読み込めるようになるため、
// 次回以降はローカルから読み込める（要件 15.3/15.4）。
//
// 要件との対応:
//
// - 要件 15.3: リモート Model_Variant の `model.onnx` とタグ定義ファイルを
//   Model_Store からローカルへダウンロードする。→ [`download_model`] が両ファイルを
//   [`ModelDownloader`] 経由で取得する。
// - 要件 15.4: ダウンロードしたモデルをローカルに保存し次回以降ローカルから読み込み
//   可能にする。→ 取得バイト列を `dest_dir/model.onnx` と `dest_dir/<tagdef>` へ
//   保存し、`dest_dir` を返す。返した `dest_dir` は [`load_local_model`] が読める
//   （`.onnx` とタグ定義の対が揃う）。
// - 要件 15.7: ダウンロードが 30 秒以内に完了しない場合は最大 3 回まで再試行し、
//   それでも失敗する場合はエラー。→ 各ファイル取得を [`DOWNLOAD_TIMEOUT`]（30 秒）
//   のタイムアウト付きで [`MAX_DOWNLOAD_ATTEMPTS`]（3 回）まで再試行し、最終失敗で
//   `Err(Download)`。
// - 要件 15.8: ローカル保存に失敗した場合はエラーとし推論機能を無効のまま維持する。
//   → 保存（I/O）失敗時は `Err`（`Io`/`AccessDenied` 等）を返し、書きかけの部分
//   ファイルを削除して「読み込めてしまう半端なモデル」を残さない（[`load_local_model`]
//   が後から成功してしまうことを防ぐ）。
//
// # 設計上の取り決め（テスト可能性のためのトランスポート分離）
//
// 実ネットワークダウンロード（HuggingFace）は非決定的で、設計方針（Testing
// Strategy / PBT を用いない領域）により単体テストでは実行しない（モックまたは
// 1〜2 例の統合で確認、100 回反復はしない）。そこでダウンロードの**トランスポート**を
// 小さなトレイト [`ModelDownloader`] の背後へ隠し、単体テストではネットワークを
// 使わないモックへ差し替えられるようにする。
//
// - [`ModelDownloader::fetch`]: リポジトリとファイル名・タイムアウトを受け取り、
//   ファイルのバイト列を返す。失敗は [`DownloadError`] で表す（タイムアウトか否かを
//   区別できる）。
// - [`download_model`]: `ModelDownloader` に対して総称で実装し、取得→保存→クリーン
//   アップの制御フロー（再試行・原子的保存・部分ファイル除去）を担う。この制御フロー
//   はモックで完全にテストできる。
// - [`HfHubDownloader`]: `hf-hub` を用いた実トランスポート。薄いラッパであり、実
//   ダウンロードの検証は統合テストに委ねる（単体テストしない）。
//
// _Requirements: 15.3, 15.4, 15.7, 15.8_

use std::time::Duration;

/// ダウンロード 1 回あたりのタイムアウト（要件 15.7: 30 秒）。
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// ダウンロードの最大試行回数（要件 15.7: 最大 3 回まで再試行）。
///
/// 「最大 3 回まで再試行」を「初回 + 再試行で合計 3 回試行する」と解釈する
/// （制御フローとしては最大 3 回 `fetch` を呼ぶ）。
const MAX_DOWNLOAD_ATTEMPTS: usize = 3;

/// ローカル保存する ONNX ファイル名。[`load_local_model`] が `.onnx` として検出する。
const SAVED_ONNX_NAME: &str = "model.onnx";

/// ダウンロードトランスポートのエラー。
///
/// タイムアウトとその他の失敗を区別する（要件 15.7 のタイムアウト再試行の判断に
/// 用いるが、[`download_model`] はどちらの失敗でも再試行する）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DownloadError {
    /// タイムアウト（既定 30 秒以内に完了しなかった）。
    Timeout,
    /// その他の失敗（ネットワーク・404・I/O 等）。メッセージを保持する。
    Other(String),
}

impl std::fmt::Display for DownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DownloadError::Timeout => write!(f, "ダウンロードがタイムアウトしました"),
            DownloadError::Other(m) => write!(f, "ダウンロードに失敗しました: {m}"),
        }
    }
}

/// モデルファイル取得のトランスポート抽象。
///
/// 実装はリポジトリ `repo` 内のファイル `file` を取得しバイト列で返す。`timeout`
/// を超えた場合は [`DownloadError::Timeout`] を返す。ネットワークや外部サービスに
/// 依存するため、単体テストではモック実装へ差し替える（設計方針）。
pub trait ModelDownloader {
    /// `repo` 内の `file` を取得する。`timeout` 超過時は [`DownloadError::Timeout`]。
    fn fetch(
        &self,
        repo: &str,
        file: &str,
        timeout: Duration,
    ) -> Result<Vec<u8>, DownloadError>;
}

/// バリアントの系統に応じたタグ定義ファイル名の候補を返す。
///
/// WD14 系は `selected_tags.csv`。ML-Danbooru 系は ONNX リポジトリの慣例に合わせ
/// `tags.csv` を試し、無ければ `.json` 系にフォールバックする。取得は先頭候補から
/// 順に試み、最初に成功したものを採用する。保存時の拡張子は採用した候補に合わせる
/// ため、[`load_local_model`] が `.csv`/`.json` として検出できる。
fn tag_definition_candidates(variant: &ModelVariant) -> &'static [&'static str] {
    match variant.family {
        ModelFamily::Wd14 => &["selected_tags.csv"],
        ModelFamily::MlDanbooru => &["tags.csv", "tags.json", "selected_tags.csv"],
        // ローカル検出モデルはダウンロード対象ではないが、防御的に候補を用意する。
        ModelFamily::Local => &["selected_tags.csv", "tags.csv", "tags.json"],
    }
}

/// リモート [`ModelVariant`] から `model.onnx` とタグ定義を取得し `dest_dir` へ保存する。
///
/// 成功時は `dest_dir` を返す。返したディレクトリには `.onnx` とタグ定義の対が
/// 揃っており、[`load_local_model`] で読み込める（要件 15.4）。
///
/// # 制御フロー
///
/// 1. `variant.location` が [`ModelLocation::Remote`] であることを確認する
///    （ローカルは対象外 → `Err(InvalidInput)`）。
/// 2. `dest_dir` を作成する（既存でも可）。
/// 3. `model.onnx` を [`fetch_with_retry`] で取得する（30 秒 × 最大 3 回）。
/// 4. タグ定義を候補名の順に取得する。いずれか 1 つが取得できればよい。
/// 5. 取得した両バイト列を原子的に（一時ファイル → リネーム）保存する。
/// 6. いずれかの段階で失敗したら、書きかけの成果物を削除してから `Err` を返す
///    （半端なモデルを残さない = 推論無効維持、要件 15.8）。
///
/// # エラー
///
/// - ローカルバリアント → `Err(InvalidInput)`。
/// - `model.onnx`/タグ定義のダウンロード最終失敗 → `Err(Download)`（要件 15.7）。
/// - 保存（ディレクトリ作成・書込・リネーム）失敗 → `Err(Io/AccessDenied)` かつ
///   部分ファイルを除去（要件 15.8）。
pub fn download_model<D: ModelDownloader>(
    downloader: &D,
    variant: &ModelVariant,
    dest_dir: &Path,
) -> AppResult<std::path::PathBuf> {
    let repo = match &variant.location {
        ModelLocation::Remote(repo) => repo.as_str(),
        ModelLocation::Local(_) => {
            return Err(AppError::invalid_input(
                "ローカルモデルはダウンロード対象ではありません",
            ));
        }
    };

    // dest_dir を用意する。作成失敗（親が無い・権限不足など）は 15.8 の保存失敗。
    if let Err(e) = std::fs::create_dir_all(dest_dir) {
        return Err(AppError::from(e)
            .with_path(dest_dir.to_string_lossy().into_owned()));
    }

    // 1) model.onnx を取得する（再試行付き）。
    let onnx_bytes = fetch_with_retry(downloader, repo, SAVED_ONNX_NAME)
        .map_err(|e| download_error(repo, SAVED_ONNX_NAME, &e))?;

    // 2) タグ定義を候補順に取得する。最初に成功した候補名で保存する。
    let candidates = tag_definition_candidates(variant);
    let mut tagdef: Option<(&'static str, Vec<u8>)> = None;
    let mut last_err: Option<DownloadError> = None;
    for &name in candidates {
        match fetch_with_retry(downloader, repo, name) {
            Ok(bytes) => {
                tagdef = Some((name, bytes));
                break;
            }
            Err(e) => last_err = Some(e),
        }
    }
    let (tagdef_name, tagdef_bytes) = match tagdef {
        Some(v) => v,
        None => {
            let e = last_err.unwrap_or(DownloadError::Other(
                "タグ定義の候補がありません".to_string(),
            ));
            return Err(download_error(
                repo,
                candidates.first().copied().unwrap_or("tag-definition"),
                &e,
            ));
        }
    };

    // 3) 取得済みバイト列を原子的に保存する。保存失敗時は書きかけを全て除去し、
    //    読み込める半端なモデルを残さない（要件 15.8）。
    let onnx_dest = dest_dir.join(SAVED_ONNX_NAME);
    let tagdef_dest = dest_dir.join(tagdef_name);

    let save_result = (|| -> AppResult<()> {
        atomic_write(&onnx_dest, &onnx_bytes)?;
        atomic_write(&tagdef_dest, &tagdef_bytes)?;
        Ok(())
    })();

    if let Err(e) = save_result {
        // 部分書込のクリーンアップ（best effort）。片方だけ残ると
        // load_local_model が「対あり」と誤検出しうるため両方除去する。
        let _ = std::fs::remove_file(&onnx_dest);
        let _ = std::fs::remove_file(&tagdef_dest);
        return Err(e);
    }

    Ok(dest_dir.to_path_buf())
}

/// [`DownloadError`] を要件 15.7 のダウンロード失敗（`AppError::Download`）へ写像する。
fn download_error(repo: &str, file: &str, e: &DownloadError) -> AppError {
    AppError::download(format!(
        "モデルファイルの取得に失敗しました（{repo}/{file}）: {e}"
    ))
    .with_path(format!("{repo}/{file}"))
}

/// 単一ファイルの取得を最大 [`MAX_DOWNLOAD_ATTEMPTS`] 回まで再試行する。
///
/// 各試行は [`DOWNLOAD_TIMEOUT`] を渡す。タイムアウト・その他の失敗いずれでも
/// 次の試行へ進み、全試行が失敗したら最後のエラーを返す（要件 15.7）。
fn fetch_with_retry<D: ModelDownloader>(
    downloader: &D,
    repo: &str,
    file: &str,
) -> Result<Vec<u8>, DownloadError> {
    let mut last_err = DownloadError::Other("試行が行われませんでした".to_string());
    for _ in 0..MAX_DOWNLOAD_ATTEMPTS {
        match downloader.fetch(repo, file, DOWNLOAD_TIMEOUT) {
            Ok(bytes) => return Ok(bytes),
            Err(e) => last_err = e,
        }
    }
    Err(last_err)
}

/// バイト列を一時ファイルへ書いてからリネームで原子的に配置する。
///
/// 部分書込を避けるため、同一ディレクトリに `<name>.download.tmp` を作って
/// 書込・flush 後に本来の名前へリネームする（Tag_File 書込と同じ流儀）。失敗時は
/// 一時ファイルを best-effort で削除する。
fn atomic_write(dest: &Path, bytes: &[u8]) -> AppResult<()> {
    use std::io::Write;

    let map_io = |e: std::io::Error| {
        AppError::from(e).with_path(dest.to_string_lossy().into_owned())
    };

    let file_name = dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("model-file");
    let tmp = match dest.parent() {
        Some(parent) => parent.join(format!("{file_name}.download.tmp")),
        None => std::path::PathBuf::from(format!("{file_name}.download.tmp")),
    };

    let write_tmp = (|| -> std::io::Result<()> {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.flush()?;
        Ok(())
    })();
    if let Err(e) = write_tmp {
        let _ = std::fs::remove_file(&tmp);
        return Err(map_io(e));
    }

    if let Err(e) = std::fs::rename(&tmp, dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(map_io(e));
    }

    Ok(())
}

/// `hf-hub` を用いた実ダウンロードトランスポート（要件 15.3 の実装）。
///
/// HuggingFace Hub のリポジトリからファイルを取得する薄いラッパ。`hf-hub` は
/// ファイルをローカルキャッシュへ取得しそのパスを返すため、本ラッパはそのパスを
/// 読み出してバイト列で返す。[`download_model`] 側で `dest_dir` へ改めて保存する
/// ことで、要件 15.4（次回以降ローカル読込）を満たすアプリ管理下の配置にする。
///
/// # テスト方針
///
/// 実ネットワークを伴うため単体テストは行わない（設計の Testing Strategy /
/// 「PBT を用いない領域」に従い、モックまたは 1〜2 例の統合テストで確認する）。
#[derive(Debug, Default, Clone)]
pub struct HfHubDownloader;

impl HfHubDownloader {
    /// 既定設定のトランスポートを生成する。
    pub fn new() -> Self {
        Self
    }
}

impl ModelDownloader for HfHubDownloader {
    fn fetch(
        &self,
        repo: &str,
        file: &str,
        _timeout: Duration,
    ) -> Result<Vec<u8>, DownloadError> {
        // hf-hub 0.4 の同期 API（`ureq` feature）を用いる。`get` はキャッシュへ
        // ダウンロードしローカルパスを返す。タイムアウトの厳密制御は hf-hub の
        // 設定に委ねる（本ラッパは薄く保つ）。
        let api = hf_hub::api::sync::ApiBuilder::new()
            .build()
            .map_err(|e| DownloadError::Other(e.to_string()))?;
        let path = api
            .model(repo.to_string())
            .get(file)
            .map_err(|e| DownloadError::Other(e.to_string()))?;
        std::fs::read(&path).map_err(|e| DownloadError::Other(e.to_string()))
    }
}

#[cfg(test)]
mod download_model_tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use tempfile::tempdir;

    /// ネットワーク非依存のモックダウンローダ。
    ///
    /// `responses` は「ファイル名 → 返すバイト列」。登録の無いファイルは
    /// [`DownloadError::Other`] を返す。`always_fail` が設定されている場合は全ての
    /// 取得を指定エラーで失敗させ、`fetch` の呼び出し回数を記録する（再試行回数の
    /// 検証用）。
    struct MockDownloader {
        responses: HashMap<String, Vec<u8>>,
        always_fail: Option<DownloadError>,
        calls: RefCell<Vec<(String, String)>>,
    }

    impl MockDownloader {
        fn with_responses(pairs: &[(&str, &[u8])]) -> Self {
            let mut responses = HashMap::new();
            for (name, bytes) in pairs {
                responses.insert((*name).to_string(), bytes.to_vec());
            }
            Self {
                responses,
                always_fail: None,
                calls: RefCell::new(Vec::new()),
            }
        }

        fn always_failing(err: DownloadError) -> Self {
            Self {
                responses: HashMap::new(),
                always_fail: Some(err),
                calls: RefCell::new(Vec::new()),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.borrow().len()
        }
    }

    impl ModelDownloader for MockDownloader {
        fn fetch(
            &self,
            repo: &str,
            file: &str,
            _timeout: Duration,
        ) -> Result<Vec<u8>, DownloadError> {
            self.calls
                .borrow_mut()
                .push((repo.to_string(), file.to_string()));
            if let Some(err) = &self.always_fail {
                return Err(err.clone());
            }
            match self.responses.get(file) {
                Some(bytes) => Ok(bytes.clone()),
                None => Err(DownloadError::Other(format!("未登録のファイル: {file}"))),
            }
        }
    }

    /// テスト用のリモート WD14 バリアント。
    fn remote_wd14() -> ModelVariant {
        ModelVariant {
            id: "wd14-vit".to_string(),
            display_name: "WD14 ViT".to_string(),
            family: ModelFamily::Wd14,
            location: ModelLocation::Remote("owner/wd14-vit".to_string()),
            onnx_available: true,
        }
    }

    #[test]
    fn successful_download_saves_files_and_dir_is_loadable_shape() {
        // 妥当な CSV タグ定義（parse_tag_definition が受理する形）を用意する。
        let csv = b"tag_id,name,category\n1,solo,0\n2,miku,4\n";
        let downloader = MockDownloader::with_responses(&[
            ("model.onnx", b"not-a-real-onnx-but-saved-verbatim"),
            ("selected_tags.csv", csv),
        ]);

        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");

        let returned = download_model(&downloader, &remote_wd14(), &dest).unwrap();
        assert_eq!(returned, dest);

        // 要件 15.4: model.onnx とタグ定義の対がローカル保存され、対として揃う。
        let onnx = dest.join("model.onnx");
        let tagdef = dest.join("selected_tags.csv");
        assert!(onnx.is_file(), "model.onnx が保存されるべき");
        assert!(tagdef.is_file(), "タグ定義が保存されるべき");
        assert_eq!(std::fs::read(&onnx).unwrap(), b"not-a-real-onnx-but-saved-verbatim");

        // 保存したタグ定義は load_local_model の解析段（parse_tag_definition）が読める。
        let labels = parse_tag_definition(&tagdef).unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].name, "solo");
        assert_eq!(labels[1].category, TagCategory::Character);

        // onnx_with_tagdef（17.1）も対として検出できる = 次回以降ローカル読込可能な形。
        assert!(super::onnx_with_tagdef(&dest).is_some());
    }

    #[test]
    fn ml_danbooru_falls_back_to_alternate_tagdef_name() {
        // ML-Danbooru は tags.csv を先頭候補にする。tags.csv を返すと採用される。
        let csv = b"name,category\nsolo,0\n";
        let downloader = MockDownloader::with_responses(&[
            ("model.onnx", b"onnx-bytes"),
            ("tags.csv", csv),
        ]);
        let variant = ModelVariant {
            id: "ml-danbooru".to_string(),
            display_name: "ML-Danbooru".to_string(),
            family: ModelFamily::MlDanbooru,
            location: ModelLocation::Remote("owner/ml-danbooru".to_string()),
            onnx_available: true,
        };

        let dir = tempdir().unwrap();
        let dest = dir.path().join("ml-danbooru");
        download_model(&downloader, &variant, &dest).unwrap();

        assert!(dest.join("model.onnx").is_file());
        assert!(dest.join("tags.csv").is_file());
    }

    #[test]
    fn download_failing_every_attempt_yields_download_error_and_retries_three_times() {
        // 全取得が失敗する。model.onnx の取得だけで最大 3 回まで再試行する。
        let downloader = MockDownloader::always_failing(DownloadError::Timeout);
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");

        let err = download_model(&downloader, &remote_wd14(), &dest).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::Download);

        // 要件 15.7: 最大 3 回試行する（model.onnx で 3 回呼ばれ、そこで打ち切る）。
        assert_eq!(downloader.call_count(), MAX_DOWNLOAD_ATTEMPTS);

        // 保存物は残らない（対が揃わない = 推論無効維持、要件 15.8 の趣旨）。
        assert!(!dest.join("model.onnx").exists());
        assert!(super::onnx_with_tagdef(&dest).is_none());
    }

    #[test]
    fn tag_definition_download_failure_yields_download_error() {
        // model.onnx は取得できるがタグ定義候補が全滅するケース。
        let downloader = MockDownloader::with_responses(&[("model.onnx", b"onnx")]);
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");

        let err = download_model(&downloader, &remote_wd14(), &dest).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::Download);
        // 対が揃わないため、読み込める半端なモデルは残らない。
        assert!(super::onnx_with_tagdef(&dest).is_none());
    }

    #[test]
    fn save_failure_leaves_no_usable_model() {
        // dest_dir としてファイル（ディレクトリではない）を指定すると、
        // create_dir_all または書込が失敗する（要件 15.8: 保存失敗）。
        let csv = b"tag_id,name,category\n1,solo,0\n";
        let downloader = MockDownloader::with_responses(&[
            ("model.onnx", b"onnx"),
            ("selected_tags.csv", csv),
        ]);

        let dir = tempdir().unwrap();
        // dest 自体をファイルにする。
        let dest_as_file = dir.path().join("not-a-dir");
        std::fs::write(&dest_as_file, b"i am a file").unwrap();

        let err = download_model(&downloader, &remote_wd14(), &dest_as_file).unwrap_err();
        // ディレクトリ作成/保存の I/O 失敗。Download ではなく保存系エラー。
        assert_ne!(err.kind, crate::error::AppErrorKind::Download);
        // ファイルはそのまま（ディレクトリ化されていない）= 使えるモデルは残らない。
        assert!(dest_as_file.is_file());
        assert!(super::onnx_with_tagdef(&dest_as_file).is_none());
    }

    #[test]
    fn local_variant_is_rejected() {
        let downloader = MockDownloader::with_responses(&[]);
        let variant = ModelVariant {
            id: "local".to_string(),
            display_name: "local".to_string(),
            family: ModelFamily::Local,
            location: ModelLocation::Local("/tmp/x/model.onnx".to_string()),
            onnx_available: true,
        };
        let dir = tempdir().unwrap();
        let err = download_model(&downloader, &variant, dir.path()).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::InvalidInput);
    }
}

// ---------------------------------------------------------------------------
// タスク 17.5: モデル異常系の単体テスト（要件 15.6 / 15.7 / 15.8 の補完）
// ---------------------------------------------------------------------------
//
// 既存の `load_local_model_tests` / `download_model_tests` が個別ケースを網羅する
// のに対し、本節は要件文言そのものを焦点に据えた補完テストを追加する:
//
// - 15.6: load_local_model が失敗した場合、LoadedModel は生成されない（推論は無効の
//   まま維持）。ONNX 読込不可とタグ定義欠落を「区別して」いずれも Err(ModelLoad)
//   になることを確認する。
// - 15.7: 再試行の境界を呼び出しカウンタで厳密に検証する。ちょうど上限回だけ失敗
//   してから成功する（=上限を超えて 1 回だけ多く成功する）ダウンローダは、上限で
//   打ち切られるため Err(Download) となり、上限内の最終試行で成功するダウンローダは
//   Ok になる。
// - 15.8: 保存に失敗した場合、使えるモデルは残らない（onnx_with_tagdef が None、
//   保存失敗後も load_local_model は Err のまま = 推論無効維持）。
//
// いずれもネットワーク非依存で、tempfile と（15.7 用の）呼び出しカウント式モックを
// 用いる。
//
// _Requirements: 15.6, 15.7, 15.8_

#[cfg(test)]
mod model_error_paths_tests {
    use super::*;
    use crate::error::AppErrorKind;
    use std::cell::Cell;
    use std::fs;
    use tempfile::tempdir;

    // -- 15.6: 読込失敗時に LoadedModel が生成されない（推論無効維持）--------

    /// ONNX 読込不可（不正バイト）とタグ定義欠落を「区別して」いずれも
    /// Err(ModelLoad) になり、LoadedModel が得られない（`is_err()`）ことを確認する。
    #[test]
    fn load_failures_never_produce_loaded_model_and_are_distinct() {
        // ケース A: タグ定義欠落（.onnx はあるがタグ定義が無い）。
        let dir_a = tempdir().unwrap();
        fs::write(dir_a.path().join("model.onnx"), b"onnx").unwrap();
        let result_a = load_local_model(dir_a.path());
        // LoadedModel は生成されない = 推論無効のまま。
        assert!(
            result_a.is_err(),
            "タグ定義欠落では LoadedModel を生成してはならない"
        );
        assert_eq!(result_a.unwrap_err().kind, AppErrorKind::ModelLoad);

        // ケース B: ONNX 読込不可（.onnx とタグ定義の対は揃うが .onnx が不正）。
        let dir_b = tempdir().unwrap();
        fs::write(dir_b.path().join("model.onnx"), b"not a real onnx").unwrap();
        fs::write(
            dir_b.path().join("selected_tags.csv"),
            b"tag_id,name,category\n1,solo,0\n",
        )
        .unwrap();
        let result_b = load_local_model(dir_b.path());
        assert!(
            result_b.is_err(),
            "ONNX 読込不可では LoadedModel を生成してはならない"
        );
        assert_eq!(result_b.unwrap_err().kind, AppErrorKind::ModelLoad);

        // 両ケースとも同じ種別（ModelLoad）で「区別された経路」を通ることを、
        // 対の有無（onnx_with_tagdef）で確認する: A は対が無い、B は対が揃う。
        assert!(
            super::onnx_with_tagdef(dir_a.path()).is_none(),
            "ケース A はタグ定義欠落で対が成立しない"
        );
        assert!(
            super::onnx_with_tagdef(dir_b.path()).is_some(),
            "ケース B は対が揃うが ONNX 段で失敗する"
        );
    }

    // -- 15.7: 再試行上限の境界をカウンタで厳密に検証 ------------------------

    /// 指定した試行番号で初めて成功するダウンローダ。
    ///
    /// `succeed_on_attempt` 回目（1 始まり）の `fetch` で `bytes` を返し、それ以前は
    /// [`DownloadError::Timeout`] を返す。全 `fetch` 呼び出しをカウントする。
    /// これにより「上限回まで失敗し続ける／上限内の最終試行で成功する」の境界を
    /// 呼び出し回数で厳密に検証できる。
    struct SucceedOnAttempt {
        succeed_on_attempt: usize,
        bytes: Vec<u8>,
        calls: Cell<usize>,
    }

    impl SucceedOnAttempt {
        fn new(succeed_on_attempt: usize, bytes: &[u8]) -> Self {
            Self {
                succeed_on_attempt,
                bytes: bytes.to_vec(),
                calls: Cell::new(0),
            }
        }

        fn call_count(&self) -> usize {
            self.calls.get()
        }
    }

    impl ModelDownloader for SucceedOnAttempt {
        fn fetch(
            &self,
            _repo: &str,
            _file: &str,
            _timeout: Duration,
        ) -> Result<Vec<u8>, DownloadError> {
            let n = self.calls.get() + 1;
            self.calls.set(n);
            if n >= self.succeed_on_attempt {
                Ok(self.bytes.clone())
            } else {
                Err(DownloadError::Timeout)
            }
        }
    }

    #[test]
    fn retry_stops_at_limit_even_if_next_attempt_would_succeed() {
        // ちょうど上限回だけ失敗し、(上限 + 1) 回目で初めて成功するはずのダウンローダ。
        // fetch_with_retry は上限回で打ち切るため、その「成功するはずの 1 回」には
        // 到達せず、Err(Download) となる（要件 15.7 の再試行上限）。
        let downloader = SucceedOnAttempt::new(
            MAX_DOWNLOAD_ATTEMPTS + 1,
            b"tag_id,name,category\n1,solo,0\n",
        );
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");

        let err = download_model(&downloader, &remote_wd14(), &dest).unwrap_err();
        assert_eq!(err.kind, AppErrorKind::Download);
        // 境界の厳密検証: model.onnx の取得でちょうど上限回だけ呼ばれて打ち切る。
        assert_eq!(downloader.call_count(), MAX_DOWNLOAD_ATTEMPTS);
        // 打ち切られたので使えるモデルは残らない。
        assert!(super::onnx_with_tagdef(&dest).is_none());
    }

    #[test]
    fn succeeds_when_last_allowed_attempt_succeeds() {
        // 上限内の最終試行（= MAX 回目）で初めて成功するダウンローダは Ok。
        // カウンタは fetch 全体で通算されるため、model.onnx の取得で MAX 回目
        // （= 通算 MAX 回目）に初成功する。その後のタグ定義取得は 1 回目の fetch で
        // 既に通算値が MAX 以上のため即成功する。よって総呼び出しは MAX + 1 回。
        let downloader = SucceedOnAttempt::new(
            MAX_DOWNLOAD_ATTEMPTS,
            b"tag_id,name,category\n1,solo,0\n",
        );
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");

        let returned = download_model(&downloader, &remote_wd14(), &dest)
            .expect("上限内の最終試行で成功するなら Ok になるべき");
        assert_eq!(returned, dest);
        // 境界の厳密検証: model.onnx が上限内の最終試行（MAX 回目）で成功し、
        // タグ定義は続く 1 回で即成功する（通算 MAX + 1 回）。
        assert_eq!(downloader.call_count(), MAX_DOWNLOAD_ATTEMPTS + 1);
        // 対が揃い、次回以降ローカル読込可能な形になっている（要件 15.4 の趣旨）。
        assert!(super::onnx_with_tagdef(&dest).is_some());
    }

    /// テスト用のリモート WD14 バリアント（download_model_tests と同じ形）。
    fn remote_wd14() -> ModelVariant {
        ModelVariant {
            id: "wd14-vit".to_string(),
            display_name: "WD14 ViT".to_string(),
            family: ModelFamily::Wd14,
            location: ModelLocation::Remote("owner/wd14-vit".to_string()),
            onnx_available: true,
        }
    }

    // -- 15.8: 保存失敗後に使えるモデルが残らない ---------------------------

    /// 常に成功するダウンローダ（保存段の失敗を切り出して検証するため）。
    struct AlwaysOkDownloader {
        onnx: Vec<u8>,
        tagdef: Vec<u8>,
    }

    impl ModelDownloader for AlwaysOkDownloader {
        fn fetch(
            &self,
            _repo: &str,
            file: &str,
            _timeout: Duration,
        ) -> Result<Vec<u8>, DownloadError> {
            if file == SAVED_ONNX_NAME {
                Ok(self.onnx.clone())
            } else {
                Ok(self.tagdef.clone())
            }
        }
    }

    #[test]
    fn save_failure_leaves_dir_unloadable_by_load_local_model() {
        // dest をファイルにして保存（ディレクトリ作成）を失敗させる（要件 15.8）。
        let downloader = AlwaysOkDownloader {
            onnx: b"onnx".to_vec(),
            tagdef: b"tag_id,name,category\n1,solo,0\n".to_vec(),
        };
        let dir = tempdir().unwrap();
        let dest_as_file = dir.path().join("occupied");
        fs::write(&dest_as_file, b"i am a file, not a dir").unwrap();

        let err = download_model(&downloader, &remote_wd14(), &dest_as_file).unwrap_err();
        // 保存系の失敗であり、ダウンロード失敗（Download）ではない。
        assert_ne!(err.kind, AppErrorKind::Download);

        // 使えるモデルは残らない: 対は成立せず、load_local_model も Err のまま。
        assert!(super::onnx_with_tagdef(&dest_as_file).is_none());
        assert!(
            load_local_model(&dest_as_file).is_err(),
            "保存失敗後は推論を有効化できてはならない"
        );
    }

    #[test]
    fn failed_download_dir_still_not_loadable_after_retry_exhaustion() {
        // ダウンロードが再試行上限まで失敗した後、そのディレクトリは
        // load_local_model で読み込めない（推論無効維持、15.7 → 15.8 の趣旨）。
        let downloader = SucceedOnAttempt::new(usize::MAX, b"never");
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");

        let err = download_model(&downloader, &remote_wd14(), &dest).unwrap_err();
        assert_eq!(err.kind, AppErrorKind::Download);
        // 保存物が無い（または対が揃わない）ので load_local_model は Err。
        assert!(
            load_local_model(&dest).is_err(),
            "ダウンロード失敗後のディレクトリは読み込めてはならない"
        );
    }
}
