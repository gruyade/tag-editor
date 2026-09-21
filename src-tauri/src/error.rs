//! 統一エラー型。
//!
//! Core の全操作は `Result<T, AppError>` を返し、Tauri コマンド境界で
//! そのまま JSON として UI へ渡す（要件 16.1, 16.2）。
//! エラーは「種別（`AppErrorKind`）＋メッセージ＋対象パス（任意）」で表現する。

use serde::{Deserialize, Serialize};
use std::fmt;

/// エラー種別。UI は種別に応じた表示を行う。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AppErrorKind {
    /// 対象が存在しない。
    NotFound,
    /// アクセス権限がない。
    AccessDenied,
    /// 宛先に同名が既に存在する（名前衝突）。
    AlreadyExists,
    /// 入力が不正（閾値・正規表現・禁止文字・batch 範囲など）。
    InvalidInput,
    /// 一般的な I/O エラー。
    Io,
    /// モデル（ONNX / タグ定義）の読込失敗。
    ModelLoad,
    /// モデルのダウンロード失敗。
    Download,
    /// 当該環境で未提供の機能（非 Windows での Windows_Only_Feature など）。
    Unsupported,
    /// ユーザー操作により処理が中断された。
    Cancelled,
}

/// アプリ統一エラー。
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppError {
    /// エラー種別。
    pub kind: AppErrorKind,
    /// 人間可読なメッセージ。
    pub message: String,
    /// エラーに関連する対象パス（任意）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
}

impl AppError {
    /// 種別とメッセージから生成。
    pub fn new(kind: AppErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            path: None,
        }
    }

    /// 対象パスを付与する。
    pub fn with_path(mut self, path: impl Into<String>) -> Self {
        self.path = Some(path.into());
        self
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::NotFound, message)
    }

    pub fn access_denied(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::AccessDenied, message)
    }

    pub fn already_exists(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::AlreadyExists, message)
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::InvalidInput, message)
    }

    pub fn io(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::Io, message)
    }

    pub fn model_load(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::ModelLoad, message)
    }

    pub fn download(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::Download, message)
    }

    pub fn unsupported(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::Unsupported, message)
    }

    pub fn cancelled(message: impl Into<String>) -> Self {
        Self::new(AppErrorKind::Cancelled, message)
    }
}

impl fmt::Display for AppError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.path {
            Some(p) => write!(f, "[{:?}] {} (path: {})", self.kind, self.message, p),
            None => write!(f, "[{:?}] {}", self.kind, self.message),
        }
    }
}

impl std::error::Error for AppError {}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        use std::io::ErrorKind as IoKind;
        let kind = match e.kind() {
            IoKind::NotFound => AppErrorKind::NotFound,
            IoKind::PermissionDenied => AppErrorKind::AccessDenied,
            IoKind::AlreadyExists => AppErrorKind::AlreadyExists,
            _ => AppErrorKind::Io,
        };
        AppError::new(kind, e.to_string())
    }
}

/// Core 内で共通利用する結果型。
pub type AppResult<T> = Result<T, AppError>;
