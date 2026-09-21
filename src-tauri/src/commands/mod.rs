//! Tauri コマンド境界（タスク 20.1、要件 16.1, 16.2, 16.7, 16.8, 17.6, 17.7）。
//!
//! 本モジュールは、各サービス（[`crate::services`]）へ薄く委譲する
//! `#[tauri::command]` アダプタ群と、長時間処理のための配線（キャンセル
//! レジストリ・進捗ブリッジ・別スレッド起動）を提供する。すべてのコマンドは
//! `Result<T, AppError>` を返し、`T`・[`crate::error::AppError`] とも
//! `serde::Serialize` を実装するため、そのまま JSON DTO としてフロントエンドへ
//! 渡せる（要件 16.1, 16.2）。
//!
//! # 構成
//!
//! - [`cancel`]: `operation_id` 別キャンセルフラグを管理する
//!   [`cancel::CancelRegistry`]（共有 `AtomicBool`、要件 17.7）。
//! - [`progress`]: 進捗通知先を抽象する [`progress::ProgressEmitter`] トレイトと
//!   テスト用の蓄積エミッタ（要件 16.7, 17.6）。
//! - [`adapters`]: 各サービスへ委譲する `#[tauri::command]` 関数群と、
//!   バッチ推論の同期コア／別スレッド起動（要件 16.8）。
//!
//! # アプリ層との境界（タスク 21 で配線）
//!
//! 実 `tauri::Builder::run` と `AppHandle::emit` は実行中のアプリ＋設定を要する。
//! 本モジュールのコマンドは関数レベルで単体テスト可能な薄いアダプタとして定義し、
//! 実 Tauri ランタイムを起動せずに委譲の正しさ・進捗系列・キャンセル反映を
//! 検証する。アプリバイナリ（タスク 21）では以下を行う。
//!
//! 1. [`cancel::CancelRegistry`] を `tauri::State` として `manage` する。
//! 2. `tauri::AppHandle` を包む [`progress::ProgressEmitter`] 実装を用意し、
//!    `emit` でフロントエンドへ進捗イベントを送出する。
//! 3. [`adapters`] のコマンドを `tauri::generate_handler!` に登録する。
//! 4. 長時間コマンド（バッチ推論・ダウンロード・大量仕訳）は
//!    [`adapters::spawn_inference_job`] のように別スレッド/タスクで起動する。

pub mod adapters;
pub mod cancel;
pub mod progress;

pub use cancel::CancelRegistry;
pub use progress::{FnEmitter, ProgressEmitter, RecordingEmitter};
