//! ModelService（タスク 3.1）: 静的カタログの構築と検証。
//!
//! 本モジュールは推論モデルの「選択肢」を静的な [`builtin_catalog`] として与える。
//! 取得状態（Model_Present / Not_Present）はカタログ定義とは分離し、Model_Dir を
//! 走査する動的判定（タスク 4.1）で別に求める。カタログは wd14-tagger 由来の
//! WD14 系各バリアントと ML-Danbooru 系（1 リポジトリ複数 `.onnx`）を取り込む。
//!
//! 要件との対応:
//!
//! - 要件 1.3/1.5: 各 [`ModelVariant`] は識別子・表示名・Model_Family を非空で保持
//!   し、WD14 系各バリアントと ML-Danbooru 系を含む。→ [`builtin_catalog`]。
//! - 要件 1.6: 1 リポジトリ複数 `.onnx` は各 `.onnx` を個別 Variant とする。→
//!   ML-Danbooru の各 `.onnx` を別 Variant として定義する。
//! - 要件 1.4: 識別子・表示名・Model_Family のいずれか欠落/空の候補は登録せず除外
//!   情報として保持する。→ [`validate_catalog`] が登録集合と除外集合へ分割する。
//! - 要件 1.7/1.8: 全 Variant にわたり識別子は一意。→ [`validate_catalog`] が重複
//!   識別子を一意化する。
//!
//! # 設計上の取り決め（純粋関数としての検証）
//!
//! [`validate_catalog`] は候補列を受け取り、`(登録集合, 除外集合)` を返す純粋関数と
//! する。これにより Property 1（必須フィールド非空）/ Property 2（登録・除外の分割）/
//! Property 3（識別子一意）が候補の組み立て方に依らず成り立つ。[`builtin_catalog`]
//! は静的候補を [`validate_catalog`] へ通した結果を返す。
//!
//! _Requirements: 1.3, 1.4, 1.5, 1.6, 1.7, 1.8_

use std::collections::HashSet;
use std::path::Path;

use crate::models::{ModelFamily, ModelSource, ModelVariant, VariantPresence};

/// 静的な Model_Catalog を返す（要件 1.3, 1.5, 1.6）。
///
/// wd14-tagger 由来の WD14 系各バリアント（`SmilingWolf/*` の v3 系ほか）と
/// ML-Danbooru 系（`deepghs/ml-danbooru-onnx` 内の複数 `.onnx`）を定義し、
/// [`validate_catalog`] を通した登録集合を返す。各 Variant は識別子・表示名・
/// Model_Family を非空で保持し（Property 1）、識別子は全体で一意（Property 3）。
///
/// WD14 系は `source.onnx_file = "model.onnx"`・`tag_files = ["selected_tags.csv"]`。
/// ML-Danbooru 系は 1 リポジトリ内の各 `.onnx` を個別 Variant とし、`onnx_file` を
/// 各 `.onnx` パスに割り当てる（要件 1.6）。
pub fn builtin_catalog() -> Vec<ModelVariant> {
    let (registered, _excluded) = validate_catalog(builtin_candidates());
    registered
}

/// [`builtin_catalog`] の元になる静的候補列を組み立てる。
///
/// この候補列を [`validate_catalog`] に通すことで、必須フィールド検証と識別子一意化
/// を経た登録集合が得られる。テストからは本関数を直接呼んで検証前の候補を得られる。
fn builtin_candidates() -> Vec<ModelVariant> {
    // WD14 系: v3 系各バリアント。onnx は "model.onnx"、タグ定義は selected_tags.csv。
    let wd14 = [
        ("wd14-vit-v3", "WD14 ViT v3", "SmilingWolf/wd-vit-tagger-v3"),
        (
            "wd14-convnext-v3",
            "WD14 ConvNeXT v3",
            "SmilingWolf/wd-convnext-tagger-v3",
        ),
        (
            "wd14-swinv2-v3",
            "WD14 SwinV2 v3",
            "SmilingWolf/wd-swinv2-tagger-v3",
        ),
        (
            "wd14-vit-large-v3",
            "WD14 ViT Large v3",
            "SmilingWolf/wd-vit-large-tagger-v3",
        ),
        (
            "wd14-eva02-large-v3",
            "WD14 EVA02 Large v3",
            "SmilingWolf/wd-eva02-large-tagger-v3",
        ),
    ];

    let mut variants: Vec<ModelVariant> = wd14
        .into_iter()
        .map(|(id, display, repo)| ModelVariant {
            id: id.to_string(),
            display_name: display.to_string(),
            family: ModelFamily::Wd14,
            source: ModelSource {
                repo: repo.to_string(),
                onnx_file: "model.onnx".to_string(),
                tag_files: vec!["selected_tags.csv".to_string()],
            },
        })
        .collect();

    // ML-Danbooru 系: 1 リポジトリ deepghs/ml-danbooru-onnx 内の複数 `.onnx` を
    // それぞれ個別 Variant として登録する（要件 1.6）。`onnx_file` を各 `.onnx`
    // パスに割り当て、`id` を区別する。
    const ML_DANBOORU_REPO: &str = "deepghs/ml-danbooru-onnx";
    let ml_danbooru = [
        (
            "ml-danbooru-caformer-dec-5-97527",
            "ML-Danbooru CAFormer",
            "ml_caformer_m36_dec-5-97527.onnx",
        ),
        (
            "ml-danbooru-tresnet-d-6-30000",
            "ML-Danbooru TResNet-D",
            "TResnet-D-FLq_ema_6-30000.onnx",
        ),
    ];

    variants.extend(
        ml_danbooru
            .into_iter()
            .map(|(id, display, onnx)| ModelVariant {
                id: id.to_string(),
                display_name: display.to_string(),
                family: ModelFamily::MlDanbooru,
                source: ModelSource {
                    repo: ML_DANBOORU_REPO.to_string(),
                    onnx_file: onnx.to_string(),
                    tag_files: vec!["tags.csv".to_string(), "tags.json".to_string()],
                },
            }),
    );

    variants
}

/// カタログ候補列を検証し、`(登録集合, 除外集合)` へ分割する純粋関数
/// （要件 1.4, 1.6, 1.7, 1.8）。
///
/// # 検証規則
///
/// 1. **必須フィールド非空**（要件 1.4）: 識別子（`id`）または表示名
///    （`display_name`）が空文字（前後トリム後に空）の候補は登録せず除外集合へ回す。
///    `family` は enum のため常に有効とみなす（design の注記）。
/// 2. **識別子一意化**（要件 1.7, 1.8）: 登録集合内で識別子が重複する場合は、
///    後続の候補にサフィックス（`-2`, `-3`, ...）を付与して一意化し、重複が 1 つも
///    残らないようにする。付与後もなお衝突する場合はさらに連番を進める。
///
/// # 戻り値
///
/// - 第 1 要素: 登録集合（識別子・表示名が非空で、識別子が一意化済み）。
/// - 第 2 要素: 除外集合（必須フィールド欠落/空。除外理由の提示に用いる、要件 1.4）。
///
/// 登録集合と除外集合の和は入力候補全体に一致する（Property 2）。
pub fn validate_catalog(candidates: Vec<ModelVariant>) -> (Vec<ModelVariant>, Vec<ModelVariant>) {
    let mut registered: Vec<ModelVariant> = Vec::new();
    let mut excluded: Vec<ModelVariant> = Vec::new();
    let mut used_ids: HashSet<String> = HashSet::new();

    for candidate in candidates {
        // 1) 必須フィールド（id / display_name）の非空検証（要件 1.4）。
        if candidate.id.trim().is_empty() || candidate.display_name.trim().is_empty() {
            excluded.push(candidate);
            continue;
        }

        // 2) 識別子の一意化（要件 1.7, 1.8）。既に使われている識別子なら
        //    サフィックスを付けて衝突しない識別子を割り当てる。
        let mut variant = candidate;
        if used_ids.contains(&variant.id) {
            let base = variant.id.clone();
            let mut suffix = 2usize;
            let mut unique = format!("{base}-{suffix}");
            while used_ids.contains(&unique) {
                suffix += 1;
                unique = format!("{base}-{suffix}");
            }
            variant.id = unique;
        }
        used_ids.insert(variant.id.clone());
        registered.push(variant);
    }

    (registered, excluded)
}

/// 指定ディレクトリ直下に `.onnx` とタグ定義（`.csv`/`.json`）が揃っていれば、
/// その `.onnx` のパスを返す。揃っていなければ `None`。
///
/// [`load_variant`] の対検出に用いる（存在判定 `is_present` でも再利用する）。
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

#[cfg(test)]
mod catalog_tests {
    use super::*;

    /// テスト用の候補を組み立てる。
    fn candidate(
        id: &str,
        display: &str,
        family: ModelFamily,
        repo: &str,
        onnx: &str,
    ) -> ModelVariant {
        ModelVariant {
            id: id.to_string(),
            display_name: display.to_string(),
            family,
            source: ModelSource {
                repo: repo.to_string(),
                onnx_file: onnx.to_string(),
                tag_files: vec!["selected_tags.csv".to_string()],
            },
        }
    }

    #[test]
    fn builtin_catalog_covers_wd14_variants_and_ml_danbooru_multiple_onnx() {
        let catalog = builtin_catalog();

        // WD14 系が複数バリアント含まれる（要件 1.5）。
        let wd14: Vec<&ModelVariant> = catalog
            .iter()
            .filter(|v| v.family == ModelFamily::Wd14)
            .collect();
        assert!(wd14.len() >= 2, "WD14 系は複数バリアントを含むべき");
        // WD14 系は model.onnx / selected_tags.csv を持つ。
        assert!(wd14.iter().all(|v| v.source.onnx_file == "model.onnx"
            && v.source
                .tag_files
                .contains(&"selected_tags.csv".to_string())));

        // ML-Danbooru 系は 1 リポジトリ内の複数 .onnx を個別 Variant として含む（要件 1.6）。
        let mld: Vec<&ModelVariant> = catalog
            .iter()
            .filter(|v| v.family == ModelFamily::MlDanbooru)
            .collect();
        assert!(
            mld.len() >= 2,
            "ML-Danbooru は複数 .onnx を個別 Variant にすべき"
        );
        // 同一リポジトリで onnx_file が相異なる。
        let repos: HashSet<&str> = mld.iter().map(|v| v.source.repo.as_str()).collect();
        assert_eq!(repos.len(), 1, "ML-Danbooru は単一リポジトリのはず");
        let onnx_files: HashSet<&str> = mld.iter().map(|v| v.source.onnx_file.as_str()).collect();
        assert_eq!(onnx_files.len(), mld.len(), "各 .onnx は相異なるべき");
    }

    #[test]
    fn builtin_catalog_fields_are_non_empty_and_ids_unique() {
        let catalog = builtin_catalog();
        // 必須フィールド非空（Property 1）。
        assert!(catalog
            .iter()
            .all(|v| !v.id.trim().is_empty() && !v.display_name.trim().is_empty()));
        // 識別子一意（Property 3）。
        let ids: HashSet<&str> = catalog.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids.len(), catalog.len(), "識別子は一意であるべき");
    }

    #[test]
    fn validate_catalog_excludes_empty_id_or_display_name() {
        let candidates = vec![
            candidate("ok", "OK", ModelFamily::Wd14, "repo/ok", "model.onnx"),
            candidate("", "空 id", ModelFamily::Wd14, "repo/x", "model.onnx"),
            candidate(
                "empty-name",
                "  ",
                ModelFamily::Wd14,
                "repo/y",
                "model.onnx",
            ),
        ];
        let total = candidates.len();
        let (registered, excluded) = validate_catalog(candidates);

        // 登録は必須フィールド非空のみ（要件 1.4）。
        assert_eq!(registered.len(), 1);
        assert_eq!(registered[0].id, "ok");
        // 除外は 2 件（空 id・空 display_name）。
        assert_eq!(excluded.len(), 2);
        // 登録 ∪ 除外 = 入力全体（Property 2）。
        assert_eq!(registered.len() + excluded.len(), total);
    }

    #[test]
    fn validate_catalog_deduplicates_conflicting_ids() {
        let candidates = vec![
            candidate("dup", "A", ModelFamily::Wd14, "repo/a", "a.onnx"),
            candidate("dup", "B", ModelFamily::Wd14, "repo/a", "b.onnx"),
            candidate("dup", "C", ModelFamily::MlDanbooru, "repo/a", "c.onnx"),
        ];
        let (registered, excluded) = validate_catalog(candidates);

        // すべて登録され（必須フィールドは非空）、識別子は一意化される（要件 1.7, 1.8）。
        assert_eq!(registered.len(), 3);
        assert!(excluded.is_empty());
        let ids: HashSet<&str> = registered.iter().map(|v| v.id.as_str()).collect();
        assert_eq!(ids.len(), 3, "一意化後に重複が残ってはならない");
        // 先頭は元の id を保持し、後続はサフィックスで区別される。
        assert_eq!(registered[0].id, "dup");
        assert!(registered[1].id.starts_with("dup-"));
        assert!(registered[2].id.starts_with("dup-"));
    }

    #[test]
    fn validate_catalog_partition_union_equals_input() {
        // 空フィールド候補と正常候補を混在させ、登録 ∪ 除外 = 入力（Property 2）。
        let candidates = vec![
            candidate("a", "A", ModelFamily::Wd14, "r/a", "a.onnx"),
            candidate("", "", ModelFamily::Wd14, "r/b", "b.onnx"),
            candidate("a", "A dup", ModelFamily::Wd14, "r/a", "a2.onnx"),
        ];
        let total = candidates.len();
        let (registered, excluded) = validate_catalog(candidates);
        assert_eq!(registered.len() + excluded.len(), total);
    }
}

// ---------------------------------------------------------------------------
// タスク 10.1: Variant 読込（load_variant）
// ---------------------------------------------------------------------------
//
// 本節は「`.onnx` とタグ定義（`.csv`/`.json`）の対」を実際に読み込み、推論に使う
// [`LoadedModel`]（ONNX セッション + ラベル定義 + 入力規約）を構築する。
//
// 要件との対応:
//
// - 要件 8.1: `variant_dir` 配下の `.onnx` と対応タグ定義を読み込んでモデルを
//   利用可能にする。→ [`load_variant`] が `.onnx` の検出・タグ定義の解析・ONNX
//   セッション構築を行い [`LoadedModel`] を返す。
// - 要件 8.2, 8.3, 8.4: ONNX 読込不可・タグ定義解析不可・書込失敗は原因を識別できる
//   エラーとし、推論を開始せず既存状態を保持する（部分的に壊れたモデルを「読み込めた」
//   ことにしない）。→ 検出・解析・セッション構築のいずれかが失敗すると
//   `Err(ModelLoad)` を返し、呼び出し側は `LoadedModel` を得られない = 推論は無効のまま。
//
// # 設計上の取り決め（テスト可能性のための分割）
//
// 実 `ort::Session` の構築は共有ライブラリ（`load-dynamic`）を要するため、単体
// テストで常に成功させられない。そこで処理を次の 2 段に分離する:
//
// 1. タグ定義の解析・検証と `.onnx` パスの特定 …… ランタイム非依存で完全にテスト可能。
//    - [`parse_tag_definition`]: `.csv`/`.json` を [`LabelDef`] 列へ解析する。
//    - [`onnx_with_tagdef`]: 対の検出に再利用する。
// 2. ONNX セッションの構築 …… 実ランタイムを要する（[`load_variant`] の後段）。
//
// これにより「タグ定義欠落」「タグ定義不正」「`.onnx` 欠落」といった要件 8.2/8.3 の異常系は
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
/// ランタイム非依存の純粋な解析で、[`load_variant`] のテスト可能な中核。
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
    .map_err(|msg| AppError::model_load(msg).with_path(path.to_string_lossy().into_owned()))?;

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
    let value: serde_json::Value =
        serde_json::from_str(content).map_err(|e| format!("JSON を解析できません: {e}"))?;

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
                    Some(serde_json::Value::Number(n)) => category_from_token(&n.to_string()),
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

/// Variant ディレクトリから [`LoadedModel`] を構築する（要件 8.1）。
///
/// `variant_dir` 直下に `.onnx` とタグ定義（`.csv`/`.json`）が対で存在することを
/// [`onnx_with_tagdef`]（対検出ロジック）で確認し、タグ定義を
/// [`parse_tag_definition`] で解析したうえで ONNX セッションを構築する。
///
/// # エラー（要件 8.2, 8.3, 8.4: いずれも推論を開始せず既存状態を保持）
///
/// - `.onnx` またはタグ定義が欠落 → `Err(ModelLoad)`。
/// - タグ定義が空/不正 → `Err(ModelLoad)`（[`parse_tag_definition`] 由来）。
/// - ONNX セッションの構築失敗（形式不正・ランタイム不在等） → `Err(ModelLoad)`。
///
/// # 入力規約
///
/// `input_size`/`channel_order` は WD14 既定（448 / BGR）を用いる。モデルメタから
/// 解決可能になった場合は後続タスクで精緻化する。
pub fn load_variant(variant_dir: &Path) -> AppResult<LoadedModel> {
    // 1) `.onnx` とタグ定義の対を検出する（対が無ければ要件 8.2 のエラー）。
    let onnx_path = onnx_with_tagdef(variant_dir).ok_or_else(|| {
        AppError::model_load("モデルディレクトリに .onnx とタグ定義（.csv/.json）の対がありません")
            .with_path(variant_dir.to_string_lossy().into_owned())
    })?;

    // 2) タグ定義ファイルを特定して解析する（欠落/不正は要件 8.3 のエラー）。
    let tagdef_path = find_tag_definition(variant_dir).ok_or_else(|| {
        AppError::model_load("タグ定義ファイル（.csv/.json）が見つかりません")
            .with_path(variant_dir.to_string_lossy().into_owned())
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

/// ONNX Runtime 共有ライブラリのファイル名（OS ごと）。
#[cfg(target_os = "windows")]
const ORT_DYLIB_FILENAME: &str = "onnxruntime.dll";
#[cfg(target_os = "linux")]
const ORT_DYLIB_FILENAME: &str = "libonnxruntime.so";
#[cfg(target_os = "macos")]
const ORT_DYLIB_FILENAME: &str = "libonnxruntime.dylib";

/// ONNX Runtime 共有ライブラリのパスを解決する。
///
/// 探索順序:
/// 1. 環境変数 `ORT_DYLIB_PATH`（CI・開発時の明示指定用。ファイルそのものへの
///    パス、またはディレクトリへのパスのいずれも受け付ける）。
/// 2. 実行ファイルと同じディレクトリ（配布時にランタイムを同梱するレイアウト）。
///
/// いずれも見つからない場合は `None` を返し、呼び出し側は `ort` 既定の
/// システム検索へフォールバックする。
fn resolve_ort_dylib_path() -> Option<std::path::PathBuf> {
    if let Ok(configured) = std::env::var("ORT_DYLIB_PATH") {
        let configured = std::path::PathBuf::from(configured);
        let candidate = if configured.is_dir() {
            configured.join(ORT_DYLIB_FILENAME)
        } else {
            configured
        };
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let candidate = exe_dir.join(ORT_DYLIB_FILENAME);
    candidate.is_file().then_some(candidate)
}

/// `ort`（ONNX Runtime、`load-dynamic`）をプロセス内で一度だけ初期化する。
///
/// `ort` は共有ライブラリを動的ロードする（`load-dynamic` 機能）。ライブラリの
/// 場所を [`ort::init_from`] で明示的に教えずに `Session::builder()` を呼ぶと、
/// ランタイムが見つからない環境（CI 等）で `ort` 内部の Mutex が poison した
/// まま unwind 不能な panic を起こしプロセスが異常終了する（Linux: SIGABRT、
/// Windows: STATUS_STACK_BUFFER_OVERRUN として観測された）。
///
/// [`resolve_ort_dylib_path`] でライブラリが見つかった場合のみ `init_from` を
/// 呼ぶ。見つからない場合は明示初期化をスキップし、`ort` 既定のシステム検索に
/// 委ねる（従来の挙動を保つ）。`OnceLock` によりプロセス内で一度だけ実行される
/// ことを保証する（`ort` の要件: 他の `ort` API 使用前に一度だけ呼ぶ）。
fn ensure_ort_initialized() {
    static INIT: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    INIT.get_or_init(|| {
        if let Some(dylib_path) = resolve_ort_dylib_path() {
            if let Ok(builder) = ort::init_from(&dylib_path) {
                builder.commit();
            }
        }
    });
}

/// `.onnx` ファイルから ONNX 推論セッションを構築する。
///
/// 実 `ort::Session` の構築で、共有ライブラリ（`load-dynamic`）を要する。構築に
/// 失敗した場合は要件 15.6 に従い `Err(ModelLoad)` を返す（推論は無効のまま）。
fn build_session(onnx_path: &Path) -> AppResult<ort::session::Session> {
    ensure_ort_initialized();
    ort::session::Session::builder()
        .and_then(|mut b| b.commit_from_file(onnx_path))
        .map_err(|e| {
            AppError::model_load(format!("ONNX セッションを構築できません: {e}"))
                .with_path(onnx_path.to_string_lossy().into_owned())
        })
}

#[cfg(test)]
mod load_variant_tests {
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
        // .onnx はあるがタグ定義が無い → 要件 8.3 のエラー（推論を開始せず既存状態を保持）。
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("model.onnx"), b"onnx").unwrap();

        let err = load_variant(dir.path()).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::ModelLoad);
    }

    #[test]
    fn missing_onnx_yields_model_load_error() {
        // タグ定義はあるが .onnx が無い → 要件 8.2 のエラー。
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("selected_tags.csv"),
            b"tag_id,name,category\n1,solo,0\n",
        )
        .unwrap();

        let err = load_variant(dir.path()).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::ModelLoad);
    }

    #[test]
    fn empty_dir_yields_model_load_error() {
        let dir = tempdir().unwrap();
        let err = load_variant(dir.path()).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::ModelLoad);
    }

    #[test]
    #[ignore = "実 ort::Session 構築（build_session）に到達する。ONNX Runtime \
        共有ライブラリのプロセス終了時解放処理はort crate側の既知の難所（環境ごとの \
        リンカセクション/dlclose順序に依存）であり、当リポジトリのCIでは制御できない。 \
        壊れたONNXバイト列を渡した際、ロード自体は失敗してAppErrorKind::ModelLoadを \
        正しく返すが、テストプロセス終了時にSIGSEGV/STATUS_STACK_BUFFER_OVERRUNで \
        異常終了する（GitHub Actions Ubuntu/Windows双方で確認済み）。ローカルで \
        ONNX Runtimeが利用可能な環境でのみ手動実行する。"]
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

        let result = load_variant(dir.path());
        // タグ定義は妥当なので、失敗するとすればセッション構築段階（要件 8.4）。
        let err = result.expect_err("不正な .onnx はモデル読込エラーになるべき");
        assert_eq!(err.kind, crate::error::AppErrorKind::ModelLoad);
    }
}

// ---------------------------------------------------------------------------
// タスク 9.1: Variant の取得・保存（download_variant）
// ---------------------------------------------------------------------------
//
// 本節は [`ModelVariant`] の `source`（[`ModelSource`]: repo + onnx_file + tag_files）
// から `.onnx` とタグ定義ファイルを取得し、`variant_dir` へ原子的にローカル保存する。
// 保存後は [`load_variant`] が `variant_dir` を読み込めるように
// なるため、次回以降はローカルから読み込める（要件 5.5）。
//
// 要件との対応:
//
// - 要件 6.1/6.2: 各ファイル取得を [`DOWNLOAD_TIMEOUT`]（30 秒）のタイムアウト付きで
//   [`MAX_DOWNLOAD_ATTEMPTS`]（3 回）まで再試行し、最終失敗で `Err(Download)`。
// - 要件 6.3: 失敗時は失敗ファイル名を含むエラーを返す（[`download_error`]）。
// - 要件 6.4: 全取得成功後に原子的確定（一時ファイル → リネーム、[`atomic_write`]）。
// - 要件 2.6/5.4/5.6/6.5/6.6: 新規（開始時 Not_Present）の失敗・キャンセルは部分
//   ファイルを除去し Not_Present へ戻す。
// - 要件 5.8: 上書き（開始時 Present）の失敗時は既存 Assets を保持する
//   （[`save_overwrite`] が退避 → 配置 → 失敗時ロールバック）。
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
// - [`download_variant`]: `ModelDownloader` に対して総称で実装し、取得→保存→クリーン
//   アップの制御フロー（再試行・原子的保存・部分ファイル除去・上書きロールバック）を
//   担う。この制御フローはモックで完全にテストできる。
// - [`HfHubDownloader`]: `hf-hub` を用いた実トランスポート。薄いラッパであり、実
//   ダウンロードの検証は統合テストに委ねる（単体テストしない）。
//
// _Requirements: 2.6, 5.4, 5.5, 5.6, 5.8, 6.1, 6.2, 6.3, 6.4, 6.5, 6.6_

use std::time::Duration;

/// ダウンロード 1 回あたりのタイムアウト（要件 15.7: 30 秒）。
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30);

/// ダウンロードの最大試行回数（要件 15.7: 最大 3 回まで再試行）。
///
/// 「最大 3 回まで再試行」を「初回 + 再試行で合計 3 回試行する」と解釈する
/// （制御フローとしては最大 3 回 `fetch` を呼ぶ）。
const MAX_DOWNLOAD_ATTEMPTS: usize = 3;

/// ダウンロードトランスポートのエラー。
///
/// タイムアウトとその他の失敗を区別する（要件 15.7 のタイムアウト再試行の判断に
/// 用いるが、`download_variant` はどちらの失敗でも再試行する）。
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
    fn fetch(&self, repo: &str, file: &str, timeout: Duration) -> Result<Vec<u8>, DownloadError>;
}

/// [`download_variant`] が取得段階の境界で通知する進捗フェーズ。
///
/// 「onnx 取得 → タグ定義取得 → 保存」の 3 段階に対応する（design 変更 4）。
/// ダウンロード起動アダプタがこれを `Progress`（`done`/`total`）へ写像して
/// フロントエンドへ emit する。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadPhase {
    /// `model.onnx` を取得完了。
    OnnxFetched,
    /// タグ定義ファイルを取得完了。
    TagDefinitionFetched,
    /// ローカル保存完了。
    Saved,
}

/// `variant.source` から `.onnx` とタグ定義を取得し `variant_dir` へ原子的に保存する
/// （要件 2.6, 5.4, 5.5, 5.6, 5.8, 6.1〜6.6）。
///
/// 旧 `download_model` / `download_model_with_progress` を一本化した唯一の取得
/// エントリ。進捗通知（`on_progress`）とキャンセル確認（`cancel`）を常に備え、
/// 進捗不要な呼び出しには空クロージャ（`|_| {}`）を渡す。
///
/// 取得元は旧 `ModelLocation` 分岐を廃し `variant.source` を用いる:
///
/// - repo = `variant.source.repo`
/// - `.onnx` の取得元ファイル名 = `variant.source.onnx_file`
/// - タグ定義候補 = `variant.source.tag_files`（先頭優先、最初に成功したものを採用）
///
/// # 保存名
///
/// `.onnx` は [`load_variant`] が拡張子 `.onnx` で検出できるよう、`source.onnx_file`
/// のベース名（拡張子 `.onnx`）で保存する（ML-Danbooru の `foo.onnx` のように
/// リポジトリ内パスを持つ場合もベース名だけを保存名にする）。タグ定義は採用した
/// 候補名で保存する（`.csv`/`.json` の拡張子が保たれる）。
///
/// # 新規 / 上書きの分岐（要件 5.8, Property 10）
///
/// 開始時に `variant_dir` が既に Model_Present（[`is_present`]）かを記録する。
///
/// - **新規（開始時 Not_Present）**: 失敗・キャンセル時は本操作が作成した部分
///   ファイルを除去し Not_Present へ戻す（要件 2.6/5.4/5.6/6.5）。
/// - **上書き（開始時 Present）**: 全取得成功後にのみ既存 `.onnx`/タグ定義を新規
///   取得分で置き換える。取得段で失敗・キャンセルした場合は既存 Assets に一切手を
///   触れず Present を維持する（要件 5.8）。保存段では既存の対を `.bak` へ退避 →
///   新規を配置 → 成功時に `.bak` を削除、いずれかの配置に失敗したら `.bak` から
///   復元してロールバックする（片方だけ壊れることを防ぐ）。
///
/// # 引数
///
/// - `downloader`: ファイル取得トランスポート（実 [`HfHubDownloader`] もモックも可）。
/// - `variant`: 取得対象。`variant.source` から repo/ファイル名/タグ定義候補を得る。
/// - `variant_dir`: 保存先ディレクトリ（[`variant_dir`] が割り当てる一意なパス）。
/// - `cancel`: キャンセル要求フラグ。取得段の境界でのみ確認する。
/// - `on_progress`: 各段階（onnx 取得 → タグ定義取得 → 保存）完了時のコールバック。
///
/// # エラー
///
/// - `.onnx`/タグ定義のダウンロード最終失敗 → `Err(Download)`（要件 6.1〜6.3）。
/// - `tag_files` が空 → `Err(Download)`（取得すべきタグ定義が定義されていない）。
/// - 保存（ディレクトリ作成・書込・リネーム）失敗 → `Err(Io/AccessDenied)`。
/// - キャンセル → `Err(Cancelled)`（要件 5.4）。
pub fn download_variant<D, F>(
    downloader: &D,
    variant: &ModelVariant,
    variant_dir: &Path,
    cancel: &std::sync::atomic::AtomicBool,
    mut on_progress: F,
) -> AppResult<std::path::PathBuf>
where
    D: ModelDownloader,
    F: FnMut(DownloadPhase),
{
    use std::sync::atomic::Ordering;

    let repo = variant.source.repo.as_str();
    let onnx_file = variant.source.onnx_file.as_str();

    // 開始時に既に Model_Present か（= 上書きか新規か）を記録する（要件 5.8）。
    let was_present = is_present(variant_dir);

    if cancel.load(Ordering::SeqCst) {
        return Err(AppError::cancelled(
            "モデルダウンロードがキャンセルされました",
        ));
    }

    // variant_dir を用意する。作成失敗（親が無い・権限不足など）は保存失敗。
    if let Err(e) = std::fs::create_dir_all(variant_dir) {
        return Err(AppError::from(e).with_path(variant_dir.to_string_lossy().into_owned()));
    }

    // 1) `.onnx` を取得する（再試行付き）。取得元は source.onnx_file。
    let onnx_bytes = fetch_with_retry(downloader, repo, onnx_file)
        .map_err(|e| download_error(repo, onnx_file, &e))?;
    on_progress(DownloadPhase::OnnxFetched);

    if cancel.load(Ordering::SeqCst) {
        // 取得段のキャンセル: まだ保存していないので既存 Assets には触れていない。
        return Err(AppError::cancelled(
            "モデルダウンロードがキャンセルされました",
        ));
    }

    // 2) タグ定義を source.tag_files の順に取得する。最初に成功した候補名で保存する。
    let candidates = &variant.source.tag_files;
    if candidates.is_empty() {
        return Err(download_error(
            repo,
            "tag-definition",
            &DownloadError::Other("タグ定義候補が定義されていません".to_string()),
        ));
    }
    let mut tagdef: Option<(String, Vec<u8>)> = None;
    let mut last_err: Option<DownloadError> = None;
    for name in candidates {
        match fetch_with_retry(downloader, repo, name) {
            Ok(bytes) => {
                tagdef = Some((name.clone(), bytes));
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
                candidates
                    .first()
                    .map(|s| s.as_str())
                    .unwrap_or("tag-definition"),
                &e,
            ));
        }
    };
    on_progress(DownloadPhase::TagDefinitionFetched);

    if cancel.load(Ordering::SeqCst) {
        return Err(AppError::cancelled(
            "モデルダウンロードがキャンセルされました",
        ));
    }

    // 3) 保存先ファイル名を決める。onnx は source.onnx_file のベース名（拡張子 .onnx）。
    let onnx_save_name = onnx_basename(onnx_file);
    let onnx_dest = variant_dir.join(&onnx_save_name);
    let tagdef_dest = variant_dir.join(&tagdef_name);

    // 取得はすべて成功済み。ここから保存段。上書き（was_present）と新規で分岐する。
    if was_present {
        save_overwrite(&onnx_dest, &onnx_bytes, &tagdef_dest, &tagdef_bytes)?;
    } else {
        let save_result = (|| -> AppResult<()> {
            atomic_write(&onnx_dest, &onnx_bytes)?;
            atomic_write(&tagdef_dest, &tagdef_bytes)?;
            Ok(())
        })();
        if let Err(e) = save_result {
            // 新規時の保存失敗: 本操作が作成した部分ファイルを除去し Not_Present へ戻す。
            let _ = std::fs::remove_file(&onnx_dest);
            let _ = std::fs::remove_file(&tagdef_dest);
            return Err(e);
        }
    }
    on_progress(DownloadPhase::Saved);

    Ok(variant_dir.to_path_buf())
}

/// リポジトリ内 `.onnx` パス（例 `sub/foo.onnx`）から保存用のベース名を取り出す。
///
/// [`load_variant`] は拡張子 `.onnx`（大小無視）で検出するため、リポジトリ内の
/// ディレクトリ構造は保存名へ持ち込まずファイル名部分のみを用いる。ファイル名が
/// 取り出せない異常時は既定の `model.onnx` にフォールバックする。
fn onnx_basename(onnx_file: &str) -> String {
    Path::new(onnx_file)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("model.onnx")
        .to_string()
}

/// 上書き保存（開始時 Model_Present）を、失敗時に既存対を保持できる形で行う
/// （要件 5.8, Property 10）。
///
/// 既存の対を `.bak` へ退避してから新規を [`atomic_write`] で配置し、両方の配置が
/// 成功したら `.bak` を削除する。いずれかで失敗した場合は `.bak` から既存を復元して
/// ロールバックし、既存 Assets（`.onnx` とタグ定義の対）を Model_Present のまま
/// 維持する。片方だけ置き換わって対が壊れることを防ぐ。
fn save_overwrite(
    onnx_dest: &Path,
    onnx_bytes: &[u8],
    tagdef_dest: &Path,
    tagdef_bytes: &[u8],
) -> AppResult<()> {
    let onnx_bak = backup_path(onnx_dest);
    let tagdef_bak = backup_path(tagdef_dest);

    // 既存を .bak へ退避（存在すれば）。退避失敗時はまだ何も壊していないので即返す。
    let onnx_backed = backup_existing(onnx_dest, &onnx_bak)?;
    let tagdef_backed = match backup_existing(tagdef_dest, &tagdef_bak) {
        Ok(v) => v,
        Err(e) => {
            // onnx の退避は済んでいるので戻す。
            if onnx_backed {
                let _ = std::fs::rename(&onnx_bak, onnx_dest);
            }
            return Err(e);
        }
    };

    // 新規を原子的に配置する。どちらかが失敗したら退避から復元してロールバックする。
    let place = (|| -> AppResult<()> {
        atomic_write(onnx_dest, onnx_bytes)?;
        atomic_write(tagdef_dest, tagdef_bytes)?;
        Ok(())
    })();

    match place {
        Ok(()) => {
            // 成功: 退避を削除して確定する。
            if onnx_backed {
                let _ = std::fs::remove_file(&onnx_bak);
            }
            if tagdef_backed {
                let _ = std::fs::remove_file(&tagdef_bak);
            }
            Ok(())
        }
        Err(e) => {
            // ロールバック: 新規に書けた分を除去し、退避から既存を復元する。
            let _ = std::fs::remove_file(onnx_dest);
            let _ = std::fs::remove_file(tagdef_dest);
            if onnx_backed {
                let _ = std::fs::rename(&onnx_bak, onnx_dest);
            }
            if tagdef_backed {
                let _ = std::fs::rename(&tagdef_bak, tagdef_dest);
            }
            Err(e)
        }
    }
}

/// `<dest>.overwrite.bak` の退避先パスを返す。
fn backup_path(dest: &Path) -> PathBuf {
    let name = dest
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("model-file");
    match dest.parent() {
        Some(parent) => parent.join(format!("{name}.overwrite.bak")),
        None => PathBuf::from(format!("{name}.overwrite.bak")),
    }
}

/// `dest` が存在すれば `bak` へリネームして退避する。退避した場合 `Ok(true)`、
/// 元から存在しなければ `Ok(false)`。リネーム失敗は `Err`。
fn backup_existing(dest: &Path, bak: &Path) -> AppResult<bool> {
    if dest.exists() {
        std::fs::rename(dest, bak)
            .map_err(|e| AppError::from(e).with_path(dest.to_string_lossy().into_owned()))?;
        Ok(true)
    } else {
        Ok(false)
    }
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

    let map_io =
        |e: std::io::Error| AppError::from(e).with_path(dest.to_string_lossy().into_owned());

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
/// 読み出してバイト列で返す。[`download_variant`] 側で `variant_dir` へ改めて保存
/// することで、要件 5.5（次回以降ローカル読込）を満たすアプリ管理下の配置にする。
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
    fn fetch(&self, repo: &str, file: &str, _timeout: Duration) -> Result<Vec<u8>, DownloadError> {
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
pub(crate) mod download_model_tests {
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
    ///
    /// `pub(crate)` として公開し、`commands::adapters` のダウンロード起動アダプタの
    /// 単体テストから実 HTTP ネットワークアクセスなしで再利用できるようにする。
    pub(crate) struct MockDownloader {
        responses: HashMap<String, Vec<u8>>,
        always_fail: Option<DownloadError>,
        calls: RefCell<Vec<(String, String)>>,
    }

    impl MockDownloader {
        pub(crate) fn with_responses(pairs: &[(&str, &[u8])]) -> Self {
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

        #[allow(dead_code)]
        pub(crate) fn always_failing(err: DownloadError) -> Self {
            Self {
                responses: HashMap::new(),
                always_fail: Some(err),
                calls: RefCell::new(Vec::new()),
            }
        }

        #[allow(dead_code)]
        pub(crate) fn call_count(&self) -> usize {
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

    use crate::models::ModelSource;
    use std::sync::atomic::AtomicBool;

    /// キャンセルされていない共有フラグ。
    fn no_cancel() -> AtomicBool {
        AtomicBool::new(false)
    }

    /// テスト用の WD14 バリアント（source ベース）。
    fn remote_wd14() -> ModelVariant {
        ModelVariant {
            id: "wd14-vit".to_string(),
            display_name: "WD14 ViT".to_string(),
            family: ModelFamily::Wd14,
            source: ModelSource {
                repo: "owner/wd14-vit".to_string(),
                onnx_file: "model.onnx".to_string(),
                tag_files: vec!["selected_tags.csv".to_string()],
            },
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
        let cancel = no_cancel();

        let returned =
            download_variant(&downloader, &remote_wd14(), &dest, &cancel, |_| {}).unwrap();
        assert_eq!(returned, dest);

        // 要件 5.5: .onnx とタグ定義の対がローカル保存され、対として揃う。
        let onnx = dest.join("model.onnx");
        let tagdef = dest.join("selected_tags.csv");
        assert!(onnx.is_file(), "model.onnx が保存されるべき");
        assert!(tagdef.is_file(), "タグ定義が保存されるべき");
        assert_eq!(
            std::fs::read(&onnx).unwrap(),
            b"not-a-real-onnx-but-saved-verbatim"
        );

        // 保存したタグ定義は load_variant の解析段（parse_tag_definition）が読める。
        let labels = parse_tag_definition(&tagdef).unwrap();
        assert_eq!(labels.len(), 2);
        assert_eq!(labels[0].name, "solo");
        assert_eq!(labels[1].category, TagCategory::Character);

        // onnx_with_tagdef も対として検出できる = 次回以降ローカル読込可能な形。
        assert!(super::onnx_with_tagdef(&dest).is_some());
    }

    #[test]
    fn uses_source_onnx_file_and_saves_by_basename() {
        // source.onnx_file がリポジトリ内パス（サブディレクトリ + 独自名）でも、
        // 取得元はそのパス、保存名はベース名（拡張子 .onnx）になる（要件 1.6 / 5.5）。
        let csv = b"name,category\nsolo,0\n";
        let downloader = MockDownloader::with_responses(&[
            ("ml_caformer_m36.onnx", b"onnx-bytes"),
            ("tags.csv", csv),
        ]);
        let variant = ModelVariant {
            id: "ml-danbooru-caformer".to_string(),
            display_name: "ML-Danbooru CAFormer".to_string(),
            family: ModelFamily::MlDanbooru,
            source: ModelSource {
                repo: "owner/ml-danbooru".to_string(),
                onnx_file: "ml_caformer_m36.onnx".to_string(),
                tag_files: vec!["tags.csv".to_string(), "tags.json".to_string()],
            },
        };

        let dir = tempdir().unwrap();
        let dest = dir.path().join("ml-danbooru-caformer");
        let cancel = no_cancel();
        download_variant(&downloader, &variant, &dest, &cancel, |_| {}).unwrap();

        // ベース名で .onnx が保存され、対が揃う。
        assert!(dest.join("ml_caformer_m36.onnx").is_file());
        assert!(dest.join("tags.csv").is_file());
        assert!(super::onnx_with_tagdef(&dest).is_some());
    }

    #[test]
    fn tag_files_fallback_to_second_candidate() {
        // tag_files の先頭が取得できない場合、2 番目の候補にフォールバックする。
        let json = br#"["solo","1girl"]"#;
        let downloader =
            MockDownloader::with_responses(&[("model.onnx", b"onnx-bytes"), ("tags.json", json)]);
        let variant = ModelVariant {
            id: "ml-danbooru".to_string(),
            display_name: "ML-Danbooru".to_string(),
            family: ModelFamily::MlDanbooru,
            source: ModelSource {
                repo: "owner/ml-danbooru".to_string(),
                onnx_file: "model.onnx".to_string(),
                tag_files: vec!["tags.csv".to_string(), "tags.json".to_string()],
            },
        };

        let dir = tempdir().unwrap();
        let dest = dir.path().join("ml-danbooru");
        let cancel = no_cancel();
        download_variant(&downloader, &variant, &dest, &cancel, |_| {}).unwrap();

        // 先頭候補 tags.csv は保存されず、採用された tags.json が保存される。
        assert!(!dest.join("tags.csv").exists());
        assert!(dest.join("tags.json").is_file());
        assert!(super::onnx_with_tagdef(&dest).is_some());
    }

    #[test]
    fn empty_tag_files_yields_download_error() {
        // tag_files が空なら取得すべきタグ定義が無く Download エラー。
        let downloader = MockDownloader::with_responses(&[("model.onnx", b"onnx")]);
        let variant = ModelVariant {
            id: "no-tagdef".to_string(),
            display_name: "No Tagdef".to_string(),
            family: ModelFamily::Wd14,
            source: ModelSource {
                repo: "owner/no-tagdef".to_string(),
                onnx_file: "model.onnx".to_string(),
                tag_files: vec![],
            },
        };
        let dir = tempdir().unwrap();
        let dest = dir.path().join("no-tagdef");
        let cancel = no_cancel();

        let err = download_variant(&downloader, &variant, &dest, &cancel, |_| {}).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::Download);
        assert!(super::onnx_with_tagdef(&dest).is_none());
    }

    #[test]
    fn download_failing_every_attempt_yields_download_error_and_retries_three_times() {
        // 全取得が失敗する。.onnx の取得だけで最大 3 回まで再試行する。
        let downloader = MockDownloader::always_failing(DownloadError::Timeout);
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        let cancel = no_cancel();

        let err =
            download_variant(&downloader, &remote_wd14(), &dest, &cancel, |_| {}).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::Download);

        // 要件 6.1/6.2: 最大 3 回試行する（.onnx で 3 回呼ばれ、そこで打ち切る）。
        assert_eq!(downloader.call_count(), MAX_DOWNLOAD_ATTEMPTS);

        // 保存物は残らない（対が揃わない = Not_Present へ戻る、要件 6.5）。
        assert!(!dest.join("model.onnx").exists());
        assert!(super::onnx_with_tagdef(&dest).is_none());
    }

    #[test]
    fn tag_definition_download_failure_yields_download_error() {
        // .onnx は取得できるがタグ定義候補が全滅するケース。
        let downloader = MockDownloader::with_responses(&[("model.onnx", b"onnx")]);
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        let cancel = no_cancel();

        let err =
            download_variant(&downloader, &remote_wd14(), &dest, &cancel, |_| {}).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::Download);
        // 対が揃わないため、読み込める半端なモデルは残らない。
        assert!(super::onnx_with_tagdef(&dest).is_none());
    }

    #[test]
    fn save_failure_leaves_no_usable_model() {
        // variant_dir としてファイル（ディレクトリではない）を指定すると、
        // create_dir_all または書込が失敗する（保存失敗）。
        let csv = b"tag_id,name,category\n1,solo,0\n";
        let downloader =
            MockDownloader::with_responses(&[("model.onnx", b"onnx"), ("selected_tags.csv", csv)]);

        let dir = tempdir().unwrap();
        // dest 自体をファイルにする。
        let dest_as_file = dir.path().join("not-a-dir");
        std::fs::write(&dest_as_file, b"i am a file").unwrap();
        let cancel = no_cancel();

        let err = download_variant(&downloader, &remote_wd14(), &dest_as_file, &cancel, |_| {})
            .unwrap_err();
        // ディレクトリ作成/保存の I/O 失敗。Download ではなく保存系エラー。
        assert_ne!(err.kind, crate::error::AppErrorKind::Download);
        // ファイルはそのまま（ディレクトリ化されていない）= 使えるモデルは残らない。
        assert!(dest_as_file.is_file());
        assert!(super::onnx_with_tagdef(&dest_as_file).is_none());
    }

    #[test]
    fn cancel_before_start_yields_cancelled() {
        // 開始前にキャンセル済みなら取得せず Cancelled。
        let downloader = MockDownloader::with_responses(&[
            ("model.onnx", b"onnx"),
            ("selected_tags.csv", b"tag_id,name,category\n1,solo,0\n"),
        ]);
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        let cancel = AtomicBool::new(true);

        let err =
            download_variant(&downloader, &remote_wd14(), &dest, &cancel, |_| {}).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::Cancelled);
        assert!(super::onnx_with_tagdef(&dest).is_none());
    }

    #[test]
    fn progress_phases_are_reported_in_order() {
        // 進捗フェーズが onnx → tagdef → saved の順で通知される。
        let csv = b"tag_id,name,category\n1,solo,0\n";
        let downloader =
            MockDownloader::with_responses(&[("model.onnx", b"onnx"), ("selected_tags.csv", csv)]);
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        let cancel = no_cancel();

        let mut phases = Vec::new();
        download_variant(&downloader, &remote_wd14(), &dest, &cancel, |p| {
            phases.push(p)
        })
        .unwrap();
        assert_eq!(
            phases,
            vec![
                DownloadPhase::OnnxFetched,
                DownloadPhase::TagDefinitionFetched,
                DownloadPhase::Saved,
            ]
        );
    }

    #[test]
    fn overwrite_success_replaces_existing_assets() {
        // 既存 Present に対する上書き成功で、新規バイト列に置き換わる（要件 5.8 の成功側）。
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("model.onnx"), b"old-onnx").unwrap();
        std::fs::write(
            dest.join("selected_tags.csv"),
            b"tag_id,name,category\n1,old,0\n",
        )
        .unwrap();
        assert!(super::is_present(&dest));

        let new_csv = b"tag_id,name,category\n1,newtag,0\n";
        let downloader = MockDownloader::with_responses(&[
            ("model.onnx", b"new-onnx"),
            ("selected_tags.csv", new_csv),
        ]);
        let cancel = no_cancel();
        download_variant(&downloader, &remote_wd14(), &dest, &cancel, |_| {}).unwrap();

        assert_eq!(std::fs::read(dest.join("model.onnx")).unwrap(), b"new-onnx");
        assert_eq!(
            std::fs::read(dest.join("selected_tags.csv")).unwrap(),
            new_csv
        );
        // 退避ファイルは残らない。
        assert!(!dest.join("model.onnx.overwrite.bak").exists());
        assert!(!dest.join("selected_tags.csv.overwrite.bak").exists());
        assert!(super::is_present(&dest));
    }

    #[test]
    fn overwrite_fetch_failure_keeps_existing_assets() {
        // 既存 Present に対する上書きで取得段が失敗すると、既存対は保持される（要件 5.8）。
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        std::fs::create_dir_all(&dest).unwrap();
        std::fs::write(dest.join("model.onnx"), b"old-onnx").unwrap();
        std::fs::write(
            dest.join("selected_tags.csv"),
            b"tag_id,name,category\n1,old,0\n",
        )
        .unwrap();
        assert!(super::is_present(&dest));

        // 取得は全失敗（.onnx すら取れない）。
        let downloader = MockDownloader::always_failing(DownloadError::Timeout);
        let cancel = no_cancel();
        let err =
            download_variant(&downloader, &remote_wd14(), &dest, &cancel, |_| {}).unwrap_err();
        assert_eq!(err.kind, crate::error::AppErrorKind::Download);

        // 既存 Assets は無傷で Present のまま（要件 5.8）。
        assert_eq!(std::fs::read(dest.join("model.onnx")).unwrap(), b"old-onnx");
        assert!(super::is_present(&dest));
    }
}

// ---------------------------------------------------------------------------
// タスク 17.5: モデル異常系の単体テスト（要件 15.6 / 15.7 / 15.8 の補完）
// ---------------------------------------------------------------------------
//
// 既存の `load_variant_tests` / `download_model_tests` が個別ケースを網羅する
// のに対し、本節は要件文言そのものを焦点に据えた補完テストを追加する:
//
// - 15.6: load_variant が失敗した場合、LoadedModel は生成されない（推論は無効の
//   まま維持）。ONNX 読込不可とタグ定義欠落を「区別して」いずれも Err(ModelLoad)
//   になることを確認する。
// - 15.7: 再試行の境界を呼び出しカウンタで厳密に検証する。ちょうど上限回だけ失敗
//   してから成功する（=上限を超えて 1 回だけ多く成功する）ダウンローダは、上限で
//   打ち切られるため Err(Download) となり、上限内の最終試行で成功するダウンローダは
//   Ok になる。
// - 15.8: 保存に失敗した場合、使えるモデルは残らない（onnx_with_tagdef が None、
//   保存失敗後も load_variant は Err のまま = 推論無効維持）。
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

    /// タグ定義欠落（.onnx はあるがタグ定義が無い）が Err(ModelLoad) になり、
    /// LoadedModel が得られないことを確認する。
    ///
    /// この経路は `onnx_with_tagdef` の対が成立しないため `build_session`
    /// （実 ort::Session 構築）へ到達せず、ONNX Runtime の有無に関わらず
    /// 安全に実行できる。
    #[test]
    fn load_failure_tag_definition_missing_never_produces_loaded_model() {
        let dir_a = tempdir().unwrap();
        fs::write(dir_a.path().join("model.onnx"), b"onnx").unwrap();
        let result_a = load_variant(dir_a.path());
        // LoadedModel は生成されない = 推論無効のまま。
        assert!(
            result_a.is_err(),
            "タグ定義欠落では LoadedModel を生成してはならない"
        );
        assert_eq!(result_a.unwrap_err().kind, AppErrorKind::ModelLoad);
        assert!(
            super::onnx_with_tagdef(dir_a.path()).is_none(),
            "タグ定義欠落で対が成立しない"
        );
    }

    /// ONNX 読込不可（.onnx とタグ定義の対は揃うが .onnx バイト列が不正）が
    /// Err(ModelLoad) になり、LoadedModel が得られないことを確認する。
    ///
    /// タグ定義欠落ケース（[`load_failure_tag_definition_missing_never_produces_loaded_model`]）
    /// とは異なり対が揃うため `build_session`（実 ort::Session 構築）へ到達する。
    #[test]
    #[ignore = "実 ort::Session 構築（build_session）に到達する。ONNX Runtime \
        共有ライブラリのプロセス終了時解放処理はort crate側の既知の難所（環境ごとの \
        リンカセクション/dlclose順序に依存）であり、当リポジトリのCIでは制御できない。 \
        壊れたONNXバイト列を渡した際、ロード自体は失敗してAppErrorKind::ModelLoadを \
        正しく返すが、テストプロセス終了時にSIGSEGV/STATUS_STACK_BUFFER_OVERRUNで \
        異常終了する（GitHub Actions Ubuntu/Windows双方で確認済み）。ローカルで \
        ONNX Runtimeが利用可能な環境でのみ手動実行する。"]
    fn load_failure_invalid_onnx_bytes_never_produces_loaded_model() {
        let dir_b = tempdir().unwrap();
        fs::write(dir_b.path().join("model.onnx"), b"not a real onnx").unwrap();
        fs::write(
            dir_b.path().join("selected_tags.csv"),
            b"tag_id,name,category\n1,solo,0\n",
        )
        .unwrap();
        let result_b = load_variant(dir_b.path());
        assert!(
            result_b.is_err(),
            "ONNX 読込不可では LoadedModel を生成してはならない"
        );
        assert_eq!(result_b.unwrap_err().kind, AppErrorKind::ModelLoad);
        assert!(
            super::onnx_with_tagdef(dir_b.path()).is_some(),
            "対が揃うが ONNX 段で失敗する"
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
        let cancel = no_cancel();

        let err =
            download_variant(&downloader, &remote_wd14(), &dest, &cancel, |_| {}).unwrap_err();
        assert_eq!(err.kind, AppErrorKind::Download);
        // 境界の厳密検証: .onnx の取得でちょうど上限回だけ呼ばれて打ち切る。
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
        let downloader =
            SucceedOnAttempt::new(MAX_DOWNLOAD_ATTEMPTS, b"tag_id,name,category\n1,solo,0\n");
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        let cancel = no_cancel();

        let returned = download_variant(&downloader, &remote_wd14(), &dest, &cancel, |_| {})
            .expect("上限内の最終試行で成功するなら Ok になるべき");
        assert_eq!(returned, dest);
        // 境界の厳密検証: .onnx が上限内の最終試行（MAX 回目）で成功し、
        // タグ定義は続く 1 回で即成功する（通算 MAX + 1 回）。
        assert_eq!(downloader.call_count(), MAX_DOWNLOAD_ATTEMPTS + 1);
        // 対が揃い、次回以降ローカル読込可能な形になっている（要件 5.5 の趣旨）。
        assert!(super::onnx_with_tagdef(&dest).is_some());
    }

    /// キャンセルされていない共有フラグ。
    fn no_cancel() -> std::sync::atomic::AtomicBool {
        std::sync::atomic::AtomicBool::new(false)
    }

    /// テスト用の WD14 バリアント（source ベース）。
    fn remote_wd14() -> ModelVariant {
        ModelVariant {
            id: "wd14-vit".to_string(),
            display_name: "WD14 ViT".to_string(),
            family: ModelFamily::Wd14,
            source: crate::models::ModelSource {
                repo: "owner/wd14-vit".to_string(),
                onnx_file: "model.onnx".to_string(),
                tag_files: vec!["selected_tags.csv".to_string()],
            },
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
            if file == "model.onnx" {
                Ok(self.onnx.clone())
            } else {
                Ok(self.tagdef.clone())
            }
        }
    }

    #[test]
    fn save_failure_leaves_dir_unloadable_by_load_variant() {
        // dest をファイルにして保存（ディレクトリ作成）を失敗させる（要件 15.8）。
        let downloader = AlwaysOkDownloader {
            onnx: b"onnx".to_vec(),
            tagdef: b"tag_id,name,category\n1,solo,0\n".to_vec(),
        };
        let dir = tempdir().unwrap();
        let dest_as_file = dir.path().join("occupied");
        fs::write(&dest_as_file, b"i am a file, not a dir").unwrap();
        let cancel = no_cancel();

        let err = download_variant(&downloader, &remote_wd14(), &dest_as_file, &cancel, |_| {})
            .unwrap_err();
        // 保存系の失敗であり、ダウンロード失敗（Download）ではない。
        assert_ne!(err.kind, AppErrorKind::Download);

        // 使えるモデルは残らない: 対は成立せず、load_variant も Err のまま。
        assert!(super::onnx_with_tagdef(&dest_as_file).is_none());
        assert!(
            load_variant(&dest_as_file).is_err(),
            "保存失敗後は推論を有効化できてはならない"
        );
    }

    #[test]
    fn failed_download_dir_still_not_loadable_after_retry_exhaustion() {
        // ダウンロードが再試行上限まで失敗した後、そのディレクトリは
        // load_variant で読み込めない（推論無効維持、15.7 → 15.8 の趣旨）。
        let downloader = SucceedOnAttempt::new(usize::MAX, b"never");
        let dir = tempdir().unwrap();
        let dest = dir.path().join("wd14-vit");
        let cancel = no_cancel();

        let err =
            download_variant(&downloader, &remote_wd14(), &dest, &cancel, |_| {}).unwrap_err();
        assert_eq!(err.kind, AppErrorKind::Download);
        // 保存物が無い（または対が揃わない）ので load_variant は Err。
        assert!(
            load_variant(&dest).is_err(),
            "ダウンロード失敗後のディレクトリは読み込めてはならない"
        );
    }
}

// ---------------------------------------------------------------------------
// タスク 2.1: Model_Dir 解決・Variant 配置・Model_Dir 作成
// ---------------------------------------------------------------------------
//
// 本節は「モデル配置フォルダ（Model_Dir）」の解決と、Variant ごとの一意な保存先
// （variant_dir）の割り当て、および Model_Dir の作成を担う。いずれも設計の
// ModelService セクションおよび Correctness Property 4 / 5 に対応する。
//
// 要件との対応:
//
// - 要件 2.1: Model_Dir をインストール基準ディレクトリ `base_dir` 配下の固定相対
//   パスとして解決し、絶対パスとして返す。→ [`resolve_model_dir`]。
// - 要件 2.3: 取得済み Model_Assets を Variant ごとに一意な配置で Model_Dir 配下へ
//   保存する。→ [`variant_dir`]（`resolve_model_dir(base_dir)/<variant.id>`）。
// - 要件 2.2: Download_Operation 開始時に Model_Dir が無ければ書き込みに先立って
//   作成する。→ [`ensure_model_dir`]。
// - 要件 2.4/2.5: Model_Dir 作成失敗・権限不足はエラー。→ [`ensure_model_dir`] が
//   `Io` / `AccessDenied`（`From<std::io::Error>` による写像）で返す。
//
// # 設計上の取り決め（決定性 / 純粋関数）
//
// `resolve_model_dir` は Property 4（同一 `base_dir` に対し常に同一の絶対パスを返し、
// 結果は `base_dir` を接頭辞に持つ固定相対パスの結合である）を満たす純粋関数とする。
// このため実ファイルシステムへの問い合わせ（`canonicalize` 等、存在に依存し非決定的）
// は行わず、`base_dir` に固定相対パス [`MODEL_DIR_RELATIVE`] を結合するだけにする。
// 実 `base_dir`（Tauri PathResolver 由来の絶対パス）はアプリ層から供給される前提。
//
// _Requirements: 2.1, 2.2, 2.3, 2.4, 2.5_

use std::path::PathBuf;

/// Model_Dir を表す、インストール基準ディレクトリからの固定相対パス（要件 2.1）。
///
/// 決定的にするため定数として固定する。`base_dir` にこの相対パスを結合したものが
/// Model_Dir となる。
const MODEL_DIR_RELATIVE: &str = "models";

/// インストール基準ディレクトリ `base_dir` から Model_Dir を解決する純粋関数。
///
/// `base_dir` に固定相対パス [`MODEL_DIR_RELATIVE`] を結合して返す。実ファイル
/// システムへ問い合わせないため、同一 `base_dir` に対し常に同一パスを返す
/// （決定性、Property 4）。`base_dir` が絶対パスであれば戻り値も絶対パスになる
/// （実運用では Tauri PathResolver が絶対パスを供給する、要件 2.1）。
pub fn resolve_model_dir(base_dir: &Path) -> PathBuf {
    base_dir.join(MODEL_DIR_RELATIVE)
}

/// Variant ごとに一意な保存先（`resolve_model_dir(base_dir)/<variant.id>`）を返す
/// 純粋関数（要件 2.3）。
///
/// `variant.id` はカタログ全体で一意（要件 1.7）なので、識別子が相異なる 2 つの
/// Variant は相異なる保存先を持つ（Property 5）。
pub fn variant_dir(base_dir: &Path, variant: &ModelVariant) -> PathBuf {
    resolve_model_dir(base_dir).join(&variant.id)
}

/// Model_Dir（またはその配下の任意のモデル配置ディレクトリ）を、未作成なら作成する
/// （要件 2.2）。
///
/// 既に存在する場合は何もしない（冪等）。中間ディレクトリも含めて作成する。
///
/// # エラー（要件 2.4/2.5）
///
/// 作成に失敗した場合は `From<std::io::Error>` の写像により、権限不足は
/// `AccessDenied`、その他の I/O 失敗は `Io`（不在の親などは `NotFound`）として
/// `Err` を返す。呼び出し側（Download_Operation）はこれを検知して中止する。
pub fn ensure_model_dir(model_dir: &Path) -> AppResult<()> {
    std::fs::create_dir_all(model_dir)?;
    Ok(())
}

#[cfg(test)]
mod model_dir_tests {
    use super::*;
    use crate::models::{ModelFamily, ModelSource};
    use tempfile::tempdir;

    /// テスト用の Variant を組み立てる。
    fn variant(id: &str) -> ModelVariant {
        ModelVariant {
            id: id.to_string(),
            display_name: id.to_string(),
            family: ModelFamily::Wd14,
            source: ModelSource {
                repo: format!("repo/{id}"),
                onnx_file: "model.onnx".to_string(),
                tag_files: vec!["selected_tags.csv".to_string()],
            },
        }
    }

    #[test]
    fn resolve_model_dir_is_deterministic() {
        // 同一入力に対し常に同一パスを返す（Property 4 の決定性）。
        let base = Path::new("/opt/tageditor");
        let a = resolve_model_dir(base);
        let b = resolve_model_dir(base);
        assert_eq!(a, b);
    }

    #[test]
    fn resolve_model_dir_is_prefixed_by_base_dir() {
        // 結果は base_dir を接頭辞に持つ固定相対パスの結合（Property 4）。
        let base = Path::new("/opt/tageditor");
        let dir = resolve_model_dir(base);
        assert!(dir.starts_with(base));
        assert_eq!(dir, base.join(MODEL_DIR_RELATIVE));
    }

    #[test]
    fn resolve_model_dir_preserves_absoluteness() {
        // 絶対パスの base_dir からは絶対パスが返る（要件 2.1）。
        // 絶対パスの表現はプラットフォーム依存（Windows はドライブレターが必要）。
        let base = if cfg!(windows) {
            Path::new("C:\\opt\\tageditor")
        } else {
            Path::new("/opt/tageditor")
        };
        assert!(resolve_model_dir(base).is_absolute());
    }

    #[test]
    fn variant_dir_is_under_model_dir_with_id() {
        let base = Path::new("/opt/tageditor");
        let v = variant("wd14-vit");
        let vdir = variant_dir(base, &v);
        assert_eq!(vdir, resolve_model_dir(base).join("wd14-vit"));
        assert!(vdir.starts_with(resolve_model_dir(base)));
    }

    #[test]
    fn distinct_ids_yield_distinct_variant_dirs() {
        // 識別子が相異なる 2 Variant の保存先は相異なる（Property 5）。
        let base = Path::new("/opt/tageditor");
        let a = variant_dir(base, &variant("wd14-vit"));
        let b = variant_dir(base, &variant("wd14-convnext"));
        assert_ne!(a, b);
    }

    #[test]
    fn ensure_model_dir_creates_missing_directory() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("models").join("wd14-vit");
        assert!(!target.exists());
        ensure_model_dir(&target).unwrap();
        assert!(target.is_dir());
    }

    #[test]
    fn ensure_model_dir_is_idempotent_when_present() {
        let dir = tempdir().unwrap();
        let target = dir.path().join("models");
        ensure_model_dir(&target).unwrap();
        // 2 回目も成功する（既存でもエラーにしない）。
        ensure_model_dir(&target).unwrap();
        assert!(target.is_dir());
    }
}

// ---------------------------------------------------------------------------
// タスク 4.1: 存在判定（is_present）とカタログ全体の取得状態（catalog_presence）
// ---------------------------------------------------------------------------
//
// 本節はカタログ定義（静的）と取得状態（動的）の分離方針に基づき、Model_Dir 配下を
// 走査して各 Variant の Model_Present / Not_Present を求める。判定は純粋な
// ファイルシステム走査で、設計の ModelService `catalog_presence` および
// Correctness Property 6 に対応する。
//
// 要件との対応:
//
// - 要件 3.2: `.onnx` と拡張子 `.csv`/`.json` のタグ定義の対が variant_dir 配下に
//   揃うときに限り Model_Present。→ [`is_present`]（[`onnx_with_tagdef`] を再利用）。
// - 要件 3.3/3.4: `.onnx` のみ・タグ定義のみは Model_Present と判定しない。
//   → [`onnx_with_tagdef`] が対の成立時のみ `Some` を返すため bool 化で満たす。
// - 要件 3.1: カタログ各 Variant について variant_dir を走査し一意な状態を返す。
//   → [`catalog_presence`]。
// - 要件 3.5/3.6: Model_Dir が未作成、または Model_Dir はあるが Assets が無い場合は
//   全件 Not_Present。→ variant_dir 不在は `read_dir` 失敗で `false`、Model_Dir
//   未作成なら各 variant_dir も当然不在で全件 `false`。
//
// # 設計上の取り決め（onnx_with_tagdef の再利用）
//
// 存在判定は [`onnx_with_tagdef`] を再利用する。同関数は variant_dir 直下を走査し、
// `.onnx` と `.csv`/`.json` タグ定義の対が揃うときのみ `Some(onnx_path)` を返す
// （拡張子は `to_ascii_lowercase` で正規化するため `.ONNX`/`.CSV` の大文字も判定
// できる）。よって [`is_present`] はその `Some`/`None` を `true`/`false` へ写すだけで
// Property 6 の同値性（対が揃う ⇔ Model_Present）を満たす。
//
// _Requirements: 3.1, 3.2, 3.3, 3.4, 3.5, 3.6_

/// `variant_dir` 配下が Model_Present か否かを判定する（要件 3.2, 3.3, 3.4）。
///
/// `variant_dir` 直下に `.onnx` と、拡張子 `.csv`/`.json`（大小無視）のタグ定義の
/// 対がともに存在するときに限り `true`。`.onnx` のみ・タグ定義のみ・ディレクトリ
/// 不在・空ディレクトリはいずれも `false`（Property 6）。
///
/// 判定は [`onnx_with_tagdef`] を再利用し、その `Some`/`None` を bool 化する。
pub fn is_present(variant_dir: &Path) -> bool {
    onnx_with_tagdef(variant_dir).is_some()
}

/// Model_Dir 配下を走査し、カタログ各 Variant の取得状態を返す（要件 3.1）。
///
/// `catalog` の各 Variant について [`variant_dir`]`(base_dir, variant)` を
/// [`is_present`] で判定し、[`VariantPresence`] を組み立てて返す。Model_Dir
/// （[`resolve_model_dir`]`(base_dir)`）が未作成なら各 variant_dir も不在となり、
/// 全件 `present = false`（Not_Present）になる（要件 3.5, 3.6）。
///
/// 戻り値は `catalog` と同順・同要素数で、各 Variant にちょうど 1 つの状態を対応
/// させる（要件 3.1 の一意な状態）。
pub fn catalog_presence(base_dir: &Path, catalog: &[ModelVariant]) -> Vec<VariantPresence> {
    catalog
        .iter()
        .map(|variant| {
            let dir = variant_dir(base_dir, variant);
            VariantPresence {
                variant: variant.clone(),
                present: is_present(&dir),
            }
        })
        .collect()
}

#[cfg(test)]
mod presence_tests {
    use super::*;
    use crate::models::{ModelFamily, ModelSource};
    use std::fs;
    use tempfile::tempdir;

    /// テスト用の Variant を組み立てる。
    fn variant(id: &str) -> ModelVariant {
        ModelVariant {
            id: id.to_string(),
            display_name: id.to_string(),
            family: ModelFamily::Wd14,
            source: ModelSource {
                repo: format!("repo/{id}"),
                onnx_file: "model.onnx".to_string(),
                tag_files: vec!["selected_tags.csv".to_string()],
            },
        }
    }

    #[test]
    fn is_present_true_when_onnx_and_tagdef_pair_exists() {
        // `.onnx` と `.csv` の対が揃う → Model_Present（要件 3.2）。
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("model.onnx"), b"onnx").unwrap();
        fs::write(
            dir.path().join("selected_tags.csv"),
            b"tag_id,name,category\n1,solo,0\n",
        )
        .unwrap();
        assert!(is_present(dir.path()));
    }

    #[test]
    fn is_present_true_with_json_tagdef() {
        // タグ定義が `.json` でも対が揃えば Model_Present（要件 3.2）。
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("model.onnx"), b"onnx").unwrap();
        fs::write(dir.path().join("tags.json"), br#"["solo"]"#).unwrap();
        assert!(is_present(dir.path()));
    }

    #[test]
    fn is_present_true_with_uppercase_extensions() {
        // 大文字拡張子（.ONNX/.CSV）も onnx_with_tagdef が正規化して判定する。
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("MODEL.ONNX"), b"onnx").unwrap();
        fs::write(
            dir.path().join("TAGS.CSV"),
            b"tag_id,name,category\n1,solo,0\n",
        )
        .unwrap();
        assert!(is_present(dir.path()));
    }

    #[test]
    fn is_present_false_when_only_onnx() {
        // `.onnx` のみ → Not_Present（要件 3.3）。
        let dir = tempdir().unwrap();
        fs::write(dir.path().join("model.onnx"), b"onnx").unwrap();
        assert!(!is_present(dir.path()));
    }

    #[test]
    fn is_present_false_when_only_tagdef() {
        // タグ定義のみ → Not_Present（要件 3.4）。
        let dir = tempdir().unwrap();
        fs::write(
            dir.path().join("selected_tags.csv"),
            b"tag_id,name,category\n1,solo,0\n",
        )
        .unwrap();
        assert!(!is_present(dir.path()));
    }

    #[test]
    fn is_present_false_when_dir_empty() {
        // 空ディレクトリ → Not_Present（要件 3.6）。
        let dir = tempdir().unwrap();
        assert!(!is_present(dir.path()));
    }

    #[test]
    fn is_present_false_when_dir_absent() {
        // ディレクトリ不在 → Not_Present（要件 3.5）。
        let dir = tempdir().unwrap();
        let absent = dir.path().join("does-not-exist");
        assert!(!is_present(&absent));
    }

    #[test]
    fn catalog_presence_marks_only_prepared_variants_present() {
        // base_dir 配下に Model_Dir を作り、片方の variant だけ対を配置する。
        let base = tempdir().unwrap();
        let catalog = vec![variant("wd14-vit"), variant("wd14-convnext")];

        let ready = variant_dir(base.path(), &catalog[0]);
        fs::create_dir_all(&ready).unwrap();
        fs::write(ready.join("model.onnx"), b"onnx").unwrap();
        fs::write(
            ready.join("selected_tags.csv"),
            b"tag_id,name,category\n1,solo,0\n",
        )
        .unwrap();

        let presence = catalog_presence(base.path(), &catalog);
        assert_eq!(presence.len(), 2);
        assert_eq!(presence[0].variant.id, "wd14-vit");
        assert!(presence[0].present, "対を配置した variant は Model_Present");
        assert_eq!(presence[1].variant.id, "wd14-convnext");
        assert!(!presence[1].present, "未配置の variant は Not_Present");
    }

    #[test]
    fn catalog_presence_all_not_present_when_model_dir_absent() {
        // Model_Dir 未作成なら全件 Not_Present（要件 3.5）。
        let base = tempdir().unwrap();
        let catalog = vec![variant("a"), variant("b"), variant("c")];
        // resolve_model_dir(base) は作成しない。
        assert!(!resolve_model_dir(base.path()).exists());

        let presence = catalog_presence(base.path(), &catalog);
        assert_eq!(presence.len(), 3);
        assert!(presence.iter().all(|p| !p.present));
    }
}
