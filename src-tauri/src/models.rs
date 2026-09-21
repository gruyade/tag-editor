//! ドメインモデル。
//!
//! UI へは JSON DTO として公開するため、原則 serde 対応とする。
//! 例外は実行時セッションを保持する [`LoadedModel`]（`ort::Session` は
//! シリアライズ不可のため serde を導出しない）。

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFamily {
    /// WD14 系（SmilingWolf 系）。
    Wd14,
    /// ML-Danbooru 系。
    MlDanbooru,
    /// ローカル検出モデル。
    Local,
}

/// モデルの所在。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum ModelLocation {
    /// ローカルパス。
    Local(String),
    /// リモートリポジトリ（HuggingFace 等）。
    Remote(String),
}

/// モデルバリアント（要件 15.1）。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ModelVariant {
    /// 一意な識別子。
    pub id: String,
    /// 表示名。
    pub display_name: String,
    /// 系統。
    pub family: ModelFamily,
    /// 所在。
    pub location: ModelLocation,
    /// ONNX 形式を持つか（false=TF 専用は一覧から除外、要件 15.5）。
    pub onnx_available: bool,
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
