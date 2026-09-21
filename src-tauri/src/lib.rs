//! TagEditor Core。
//!
//! ファイル操作・タグ処理・ONNX 推論を担う Rust ネイティブ層。
//! UI 層とは Tauri コマンド境界（後続タスク）でのみ通信する。
//!
//! 本クレートの構成:
//! - [`error`]: 統一エラー型 `AppError`。
//! - [`models`]: serde 対応のドメインモデル。
//! - [`logic`]: I/O に依存しない純粋ロジック（後続タスクで実装）。
//! - [`services`]: ファイル I/O・推論・モデル取得など副作用を伴うサービス層。
//! - [`commands`]: 各サービスへ委譲する Tauri コマンド境界（アダプタ・キャンセル
//!   レジストリ・進捗ブリッジ）。

pub mod app;
pub mod commands;
pub mod error;
pub mod logic;
pub mod models;
pub mod services;

pub use app::run;

pub use error::{AppError, AppErrorKind, AppResult};
pub use models::{
    ChannelOrder, ImageEntry, ImageTagResult, LabelDef, LoadedModel, ModelFamily, ModelLocation,
    ModelVariant, OperationReport, Progress, SortingOperation, Tag, TagCategory, TagCount,
};
