//! ドメインモデル。
//!
//! UI へは JSON DTO として公開するため、原則 serde 対応とする。
//! 例外は実行時セッションを保持する [`LoadedModel`]（`ort::Session` は
//! シリアライズ不可のため serde を導出しない）。

use std::collections::HashSet;

use regex::Regex;
use serde::{Deserialize, Serialize};

/// 画像エントリ（要件 1）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageEntry {
    /// 画像ファイルの絶対パス。
    pub path: String,
    /// ファイル名（拡張子込み）。
    pub file_name: String,
    /// 同名の Tag_File が存在するか。
    pub has_tag_file: bool,
    /// サムネイル生成が可能か（破損等で不可なら false、要件 1.6）。
    pub thumbnail_available: bool,
}

/// タグ。信頼度は任意（要件 5.3）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tag {
    /// 正規化本体（前後トリム済み）。
    pub body: String,
    /// 信頼度 0.0〜1.0。無い場合は `None`。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
}

impl Tag {
    /// 信頼度なしのタグを生成。
    pub fn new(body: impl Into<String>) -> Self {
        Self {
            body: body.into(),
            confidence: None,
        }
    }

    /// 信頼度付きのタグを生成。
    pub fn with_confidence(body: impl Into<String>, confidence: f32) -> Self {
        Self {
            body: body.into(),
            confidence: Some(confidence),
        }
    }
}

/// タグ集計結果の 1 件（要件 6.1）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TagCount {
    /// タグ名。
    pub tag: String,
    /// 出現回数。
    pub count: usize,
}

/// 仕訳種別（要件 7.1）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SortingOperation {
    /// サブフォルダ名を接頭辞にして単一宛先へ集約。
    Gather,
    /// 接頭辞/元名に分解して接頭辞名サブフォルダへ配置。
    Distribute,
    /// 移動。
    Move,
    /// コピー。
    Copy,
}

/// 各種一括/仕訳の共通結果（要件 3.7, 7.8, 9, 10）。
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct OperationReport {
    /// 成功件数。
    pub succeeded: usize,
    /// 名前衝突で除外した件数。
    pub conflicted: usize,
    /// 失敗/対象外で除外した件数。
    pub skipped: usize,
    /// 付随メッセージ。
    pub messages: Vec<String>,
}

/// モデルの系統。
///
/// 【移行】`Local` バリアントを廃止（ローカル検出モデルの概念を廃止し、
/// リモート取得 → ローカル保存 → ローカル読込に一本化）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFamily {
    /// WD14 系（SmilingWolf 系）。
    Wd14,
    /// ML-Danbooru 系。
    MlDanbooru,
}

/// モデルの取得元（Model_Source、要件 7.2）。
///
/// 取得（Download_Operation）専用で推論には用いない。
/// 【移行】既存 `ModelLocation { Local, Remote }` を置き換える。
/// Remote を推論経路へ混ぜる余地を型レベルで排除する。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelSource {
    /// HuggingFace repo_id（例 `"SmilingWolf/wd-vit-tagger-v3"`）。
    pub repo: String,
    /// リポジトリ内の `.onnx` パス（1 リポジトリ複数 onnx を区別、要件 1.6）。
    pub onnx_file: String,
    /// タグ定義候補（先頭優先、例 `["selected_tags.csv"]`）。
    pub tag_files: Vec<String>,
}

/// モデルバリアント（カタログ登録の個別モデル定義、要件 1.3）。
///
/// 【移行】`location` / `onnx_available` を廃止し `source` を導入。
/// 推論は常に `variant_dir` に対して行うため（要件 7.1）、この定義は
/// 取得と表示のためだけに用いる。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelVariant {
    /// 一意な識別子（非空、要件 1.3/1.7）。
    pub id: String,
    /// 表示名（非空、要件 1.3）。
    pub display_name: String,
    /// 系統（要件 1.3）。
    pub family: ModelFamily,
    /// 取得元（推論では未使用、要件 7.2）。
    pub source: ModelSource,
}

/// Model_Present / Not_Present の動的判定結果（要件 3.1）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VariantPresence {
    /// 対象バリアント。
    pub variant: ModelVariant,
    /// `true`=Model_Present、`false`=Not_Present（要件 3.2）。
    pub present: bool,
}

/// `list_catalog` の戻り値（要件 1.1/1.2/3.5）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogListing {
    /// カタログ各バリアントと取得状態。
    pub variants: Vec<VariantPresence>,
    /// 必須フィールド欠落で除外した情報（要件 1.4）。
    pub excluded: Vec<ModelVariant>,
    /// Model_Dir が存在するか（要件 3.5）。
    pub model_dir_present: bool,
}

/// タグのカテゴリ（selected_tags.csv/json 由来）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TagCategory {
    /// レーティング。
    Rating,
    /// 一般タグ。
    General,
    /// キャラクタ。
    Character,
}

/// ラベル定義（要件 14, 15）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LabelDef {
    /// タグ名。
    pub name: String,
    /// カテゴリ。
    pub category: TagCategory,
}

/// 入力チャンネル順（モデルメタから解決）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChannelOrder {
    /// RGB 順。
    Rgb,
    /// BGR 順（WD14 系の既定）。
    Bgr,
}

/// 推論 1 画像の結果（要件 14）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageTagResult {
    /// 画像パス。
    pub image_path: String,
    /// 閾値適用前の全確信度。採用は呼び出し側で判定。
    pub tags: Vec<Tag>,
}

/// 進捗イベントペイロード（要件 16.7, 17.6）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Progress {
    /// 対象処理の識別子。
    pub operation_id: String,
    /// 処理済み件数。
    pub done: usize,
    /// 総件数。
    pub total: usize,
}

/// 読み込み済みモデル。
///
/// 実行時セッション（`ort::Session`）を保持するため serde は導出しない。
/// UI へはこのモデル自体を返さず、[`ModelVariant`] などの DTO を返す。
pub struct LoadedModel {
    /// ONNX 推論セッション。
    pub session: ort::session::Session,
    /// 入力の一辺サイズ（例 448）。
    pub input_size: u32,
    /// 入力チャンネル順。
    pub channel_order: ChannelOrder,
    /// ラベル定義（selected_tags.csv/json 由来）。
    pub labels: Vec<LabelDef>,
}

impl std::fmt::Debug for LoadedModel {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LoadedModel")
            .field("input_size", &self.input_size)
            .field("channel_order", &self.channel_order)
            .field("labels", &self.labels.len())
            .finish_non_exhaustive()
    }
}

// --- Tag_Filter（新規、要件 9, 10） ---

/// UI から受け取る生のフィルタ設定（DTO 境界、正規表現は未コンパイル）。
///
/// `TagFilter` は `regex::Regex` を保持し serde 非導出のため、UI との
/// 受け渡しはこの `RawTagFilter` を用いる。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RawTagFilter {
    /// Keep_Tags（正規化キー、要件 9.3）。
    pub keep: Vec<String>,
    /// Exclude_Rules の検索パターン（要件 9.4）。
    pub exclude: Vec<String>,
    /// Replace_Rules（検索パターン, 置換）（要件 9.5）。
    pub replace: Vec<(String, String)>,
    /// Additional_Tags（要件 9.6）。
    pub additional: Vec<String>,
    /// Confidence_Threshold 0.0〜1.0（要件 9.2）。
    pub confidence_threshold: f32,
    /// Fraction_Threshold 0.0〜1.0（要件 10.1、0 で無効 10.3）。
    pub fraction_threshold: f32,
}

/// コンパイル済みフィルタ（無効パターンを除外済み）。
///
/// `regex::Regex` を保持するため serde を導出しない。DTO 境界では
/// [`RawTagFilter`] を用いる。
#[derive(Debug, Clone)]
pub struct TagFilter {
    /// 正規化キー集合。
    pub keep: HashSet<String>,
    /// `^...$`・大小無視でコンパイル済みの除外パターン。
    pub exclude: Vec<Regex>,
    /// `^...$`・大小無視でコンパイル済みの置換パターンと置換文字列。
    pub replace: Vec<(Regex, String)>,
    /// Additional_Tags。
    pub additional: Vec<String>,
    /// Confidence_Threshold 0.0〜1.0。
    pub confidence_threshold: f32,
    /// Fraction_Threshold 0.0〜1.0。
    pub fraction_threshold: f32,
}

/// 無効パターン通知（要件 9.7、UI へ `InvalidInput` として提示）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InvalidPattern {
    /// 無効だった検索パターン。
    pub pattern: String,
    /// 無効と判定した理由。
    pub reason: String,
}

/// 単一画像のフィルタ結果（要件 9）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FilterOutcome {
    /// Adopted_Tags（確信度保持、代表確信度算出に使う）。
    pub adopted: Vec<Tag>,
    /// Discarded_Tags。
    pub discarded: Vec<Tag>,
}

/// バッチ集計後の結果（Fraction_Threshold 適用後、要件 10）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BatchOutcome {
    /// 画像ごとの最終採用/不採用（書込対象）。
    pub per_image: Vec<FilterOutcome>,
    /// 一覧提示用の集計。
    pub overview: TagOverview,
}

/// Tag_Overview: バッチ後のタグ一覧（要件 11.1/11.2）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagOverview {
    /// 採用タグ。
    pub adopted: Vec<TagStat>,
    /// 不採用タグ。
    pub discarded: Vec<TagStat>,
}

/// 一覧の 1 タグ（代表確信度付き、要件 11.2）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TagStat {
    /// タグ名。
    pub name: String,
    /// 出現画像の確信度平均。
    pub representative_confidence: f32,
    /// 出現画像数（割合算出の根拠）。
    pub image_count: usize,
}

/// バッチ推論の結果（既存 `InferResult` を拡張）。
///
/// 【移行】既存 `InferResult` の succeeded/failed/excluded/cancelled/messages は
/// 不変。`overview` を追加する（要件 11.1）。
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InferBatchResult {
    /// Tag_File 書込まで成功した件数。
    pub succeeded: usize,
    /// 読込不可・推論失敗・書込失敗でスキップした件数。
    pub failed: usize,
    /// mp4 除外により推論対象から外した件数。
    pub excluded: usize,
    /// キャンセルにより未処理のまま中止した件数。
    pub cancelled: usize,
    /// 付随メッセージ（失敗理由など）。
    pub messages: Vec<String>,
    /// バッチ後のタグ一覧（要件 11.1）。
    pub overview: TagOverview,
}
