//! サービス層。
//!
//! ファイル I/O・推論・モデル取得など、副作用を伴う操作を提供する。
//! 純粋ロジック（[`crate::logic`]）を土台に、実ファイルシステムや外部リソース
//! への操作を組み立てる。Tauri コマンド境界（後続タスク）はこれらのサービスへ
//! 薄く委譲する。

pub mod caption_service; // タスク 13.5: 孤立キャプション特定・削除のファイル I/O 結線（要件 11.1〜11.4）
pub mod file_service; // タスク 10/11: 列挙・サムネイル・プレビュー・Tag_File 読み書き
pub mod inference_service; // タスク 16.1: 前処理と ort セッション推論（要件 14.1）
pub mod model_service; // タスク 17.1: モデル一覧と ONNX フィルタ（要件 15.1, 15.5）
pub mod platform_service;
pub mod rename_service; // タスク 13: 正規表現置換・連番改名のファイル I/O 結線（要件 8.1〜8.6）
pub mod sort_service; // タスク 14: 仕訳（gather/distribute/move/copy）のファイル I/O 結線（要件 7.1〜7.8）
pub mod tag_file; // タスク 11: Tag_File 読み込み・書き込み（要件 2.1〜2.6）
pub mod tag_service; // タスク 12: 一括追加・削除・重複除去のファイル I/O 結線（要件 3.1〜3.8, 12） // タスク 18.1: capabilities とシンボリックリンク作成（要件 13.1〜13.6, 16.6）
