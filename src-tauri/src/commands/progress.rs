//! 進捗ブリッジ（要件 16.7, 17.6）。
//!
//! 長時間処理は 1 件処理するごとに [`Progress`] を通知する。Core の処理関数
//! （[`crate::services::inference_service::run_inference`] など）は Tauri に依存せず
//! `&mut dyn FnMut(Progress)` を受け取る設計であり、通知先を差し替え可能にする。
//!
//! 本モジュールは通知先を抽象する [`ProgressEmitter`] トレイトを定義する。
//!
//! - アプリ層（タスク 21 のバイナリ配線）では `tauri::AppHandle` を包む実装が
//!   [`ProgressEmitter::emit`] 内で `AppHandle::emit` を呼び、フロントエンドへ
//!   イベントを送出する。実 `emit` は実行中のアプリを要するため、この実装は
//!   アプリバイナリ側に置く。
//! - 単体テストでは通知を [`Vec`] に蓄積するモック実装（[`RecordingEmitter`]）を
//!   用い、実行中のアプリなしで進捗系列を検証できる。
//!
//! [`ProgressEmitter::as_callback`] は、`&mut dyn FnMut(Progress)` を要求する
//! Core 関数へエミッタを橋渡しするためのアダプタを返す。

use crate::models::Progress;

/// 進捗通知先の抽象。
///
/// 実装は 1 件の [`Progress`] を受け取り、任意の手段（Tauri イベント送出・
/// ログ・テスト用の蓄積など）で通知する。`emit` は処理ループから高頻度に
/// 呼ばれ得るため、失敗しても処理自体は継続させる（戻り値を持たせず、実装側で
/// 送出失敗を握りつぶすか記録する）設計とする。
pub trait ProgressEmitter {
    /// 1 件分の進捗を通知する。
    fn emit(&mut self, progress: Progress);

    /// `&mut dyn FnMut(Progress)` を要求する Core 関数へ渡すためのクロージャを返す。
    ///
    /// 返り値は本エミッタを可変借用するクロージャで、`run_inference` などの
    /// `progress: &mut dyn FnMut(Progress)` 引数へそのまま渡せる。
    fn as_callback(&mut self) -> Box<dyn FnMut(Progress) + '_>
    where
        Self: Sized,
    {
        Box::new(move |p: Progress| self.emit(p))
    }
}

/// 任意の `FnMut(Progress)` を [`ProgressEmitter`] として扱うアダプタ。
///
/// 既存のクロージャ資産をエミッタ境界へ持ち込むために用いる。
pub struct FnEmitter<F: FnMut(Progress)>(pub F);

impl<F: FnMut(Progress)> ProgressEmitter for FnEmitter<F> {
    fn emit(&mut self, progress: Progress) {
        (self.0)(progress)
    }
}

/// 通知された進捗を順に蓄積するテスト用エミッタ。
///
/// 実行中のアプリなしで進捗系列（`done`/`total` の推移や `operation_id`）を
/// 検証するために用いる。
#[derive(Debug, Default)]
pub struct RecordingEmitter {
    /// 受け取った進捗を受信順に保持する。
    pub events: Vec<Progress>,
}

impl RecordingEmitter {
    /// 空のレコーダを生成する。
    pub fn new() -> Self {
        Self { events: Vec::new() }
    }
}

impl ProgressEmitter for RecordingEmitter {
    fn emit(&mut self, progress: Progress) {
        self.events.push(progress);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recording_emitter_collects_events_in_order() {
        let mut rec = RecordingEmitter::new();
        rec.emit(Progress {
            operation_id: "op".into(),
            done: 1,
            total: 3,
        });
        rec.emit(Progress {
            operation_id: "op".into(),
            done: 2,
            total: 3,
        });
        assert_eq!(rec.events.len(), 2);
        assert_eq!(rec.events[0].done, 1);
        assert_eq!(rec.events[1].done, 2);
        assert_eq!(rec.events[1].total, 3);
    }

    #[test]
    fn as_callback_bridges_to_fnmut_consumer() {
        // Core 関数を模した、`&mut dyn FnMut(Progress)` を受け取る消費側。
        fn drive(total: usize, progress: &mut dyn FnMut(Progress)) {
            for done in 1..=total {
                progress(Progress {
                    operation_id: "job".into(),
                    done,
                    total,
                });
            }
        }

        let mut rec = RecordingEmitter::new();
        {
            let mut cb = rec.as_callback();
            drive(3, &mut *cb);
        }
        // エミッタへ 3 件蓄積され、done が単調増加している。
        let dones: Vec<usize> = rec.events.iter().map(|p| p.done).collect();
        assert_eq!(dones, vec![1, 2, 3]);
    }

    #[test]
    fn fn_emitter_wraps_closure() {
        let mut count = 0usize;
        {
            let mut emitter = FnEmitter(|_p: Progress| count += 1);
            emitter.emit(Progress {
                operation_id: "x".into(),
                done: 1,
                total: 1,
            });
        }
        assert_eq!(count, 1);
    }
}
