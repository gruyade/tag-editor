//! キャンセルレジストリ（要件 17.7）。
//!
//! 長時間処理（バッチ推論・大量仕訳・ダウンロード）は `operation_id` で識別し、
//! 各処理ごとに共有 [`AtomicBool`] のキャンセルフラグを持たせる。UI は
//! `cancel_operation` コマンド（[`super::adapters::cancel_operation`]）を通じて
//! 該当 `operation_id` のフラグを立て、処理側はバッチ境界などでフラグを確認して
//! 未処理を中止する（[`crate::services::inference_service::run_inference`] 参照）。
//!
//! # Tauri 非依存の設計
//!
//! レジストリは [`Mutex`]`<`[`HashMap`]`>` を包む素朴な構造体であり、実行中の
//! Tauri アプリを一切必要としない。したがってコマンド境界の配線とは独立に
//! 単体テスト可能で、スレッド間共有（`Arc<AtomicBool>` の複製）を通じて
//! 生産スレッドと消費スレッドが同じフラグを観測できることを検証できる。
//!
//! アプリ層（タスク 21 のバイナリ配線）では、この [`CancelRegistry`] を
//! `tauri::State` として管理し、各長時間コマンドが処理開始時に
//! [`CancelRegistry::register`] でフラグを取得、完了時に
//! [`CancelRegistry::clear`] で解放する。

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

/// `operation_id` → キャンセルフラグの対応を保持する共有レジストリ。
///
/// 内部は [`Mutex`] で保護した [`HashMap`]。各値は [`Arc`]`<`[`AtomicBool`]`>` で、
/// 登録側（処理スレッド）とキャンセル要求側（UI コマンド）が同一フラグを共有する。
#[derive(Debug, Default)]
pub struct CancelRegistry {
    inner: Mutex<HashMap<String, Arc<AtomicBool>>>,
}

impl CancelRegistry {
    /// 空のレジストリを生成する。
    pub fn new() -> Self {
        Self {
            inner: Mutex::new(HashMap::new()),
        }
    }

    /// `operation_id` に対するキャンセルフラグを登録し、その共有ハンドルを返す。
    ///
    /// 同一 `operation_id` が既に登録済みの場合は、新しい（false 初期化済みの）
    /// フラグで置き換える。これは同じ識別子で処理を再開する場合に前回の
    /// キャンセル状態を持ち越さないための仕様。戻り値の [`Arc`] を処理側へ渡し、
    /// バッチ境界で [`AtomicBool::load`] を確認する。
    pub fn register(&self, operation_id: impl Into<String>) -> Arc<AtomicBool> {
        let flag = Arc::new(AtomicBool::new(false));
        let mut map = self.inner.lock().expect("cancel registry poisoned");
        map.insert(operation_id.into(), Arc::clone(&flag));
        flag
    }

    /// `operation_id` のキャンセルを要求する（フラグを true にする）。
    ///
    /// 登録が存在すれば `true`、未登録なら `false` を返す。未登録の場合は
    /// 何もしない（既に完了/未開始の処理へのキャンセルは無害）。
    pub fn request_cancel(&self, operation_id: &str) -> bool {
        let map = self.inner.lock().expect("cancel registry poisoned");
        match map.get(operation_id) {
            Some(flag) => {
                flag.store(true, Ordering::SeqCst);
                true
            }
            None => false,
        }
    }

    /// `operation_id` がキャンセル要求済みか判定する。
    ///
    /// 未登録の場合は `false`（キャンセルされていない）を返す。
    pub fn is_cancelled(&self, operation_id: &str) -> bool {
        let map = self.inner.lock().expect("cancel registry poisoned");
        map.get(operation_id)
            .map(|flag| flag.load(Ordering::SeqCst))
            .unwrap_or(false)
    }

    /// `operation_id` の登録を解放する（処理完了時に呼ぶ）。
    ///
    /// 登録が存在して削除できた場合は `true`、未登録なら `false` を返す。
    pub fn clear(&self, operation_id: &str) -> bool {
        let mut map = self.inner.lock().expect("cancel registry poisoned");
        map.remove(operation_id).is_some()
    }

    /// 現在登録されている処理数（主にテスト・診断用）。
    pub fn len(&self) -> usize {
        self.inner.lock().expect("cancel registry poisoned").len()
    }

    /// 登録が空か（主にテスト・診断用）。
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn register_returns_flag_initially_false() {
        let reg = CancelRegistry::new();
        let flag = reg.register("op1");
        assert!(!flag.load(Ordering::SeqCst));
        assert!(!reg.is_cancelled("op1"));
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn request_cancel_sets_flag_observed_via_registry_and_handle() {
        let reg = CancelRegistry::new();
        let flag = reg.register("op1");

        assert!(reg.request_cancel("op1"));
        // レジストリ経由でも共有ハンドル経由でも同じ true を観測できる。
        assert!(reg.is_cancelled("op1"));
        assert!(flag.load(Ordering::SeqCst));
    }

    #[test]
    fn request_cancel_unknown_id_is_false_and_noop() {
        let reg = CancelRegistry::new();
        assert!(!reg.request_cancel("missing"));
        assert!(!reg.is_cancelled("missing"));
    }

    #[test]
    fn clear_removes_registration() {
        let reg = CancelRegistry::new();
        reg.register("op1");
        assert!(reg.clear("op1"));
        // 解放後は未登録扱い。
        assert!(!reg.is_cancelled("op1"));
        assert!(!reg.clear("op1"));
        assert!(reg.is_empty());
    }

    #[test]
    fn re_register_resets_previous_cancel_state() {
        let reg = CancelRegistry::new();
        let first = reg.register("op1");
        reg.request_cancel("op1");
        assert!(first.load(Ordering::SeqCst));

        // 同一 id で再登録すると新しい false フラグに置き換わる。
        let second = reg.register("op1");
        assert!(!second.load(Ordering::SeqCst));
        assert!(!reg.is_cancelled("op1"));
        // 古いハンドルは true のまま（別インスタンス）。
        assert!(first.load(Ordering::SeqCst));
    }

    #[test]
    fn multiple_operations_are_independent() {
        let reg = CancelRegistry::new();
        reg.register("a");
        reg.register("b");
        reg.request_cancel("a");
        assert!(reg.is_cancelled("a"));
        assert!(!reg.is_cancelled("b"));
    }

    /// スレッド安全性: 別スレッドがキャンセルを要求し、処理スレッドが共有
    /// フラグでそれを観測できる（要件 17.7 のキャンセル配線）。
    #[test]
    fn cancel_is_observable_across_threads() {
        let reg = Arc::new(CancelRegistry::new());
        // 処理スレッドが保持する共有フラグ。
        let flag = reg.register("job");

        // 別スレッドからキャンセルを要求する。
        let reg_for_canceller = Arc::clone(&reg);
        let canceller = thread::spawn(move || {
            reg_for_canceller.request_cancel("job");
        });
        canceller.join().unwrap();

        // 処理スレッド側の共有ハンドルで true を観測できる。
        assert!(flag.load(Ordering::SeqCst));
        assert!(reg.is_cancelled("job"));
    }

    /// 多数のスレッドから同一処理へキャンセル要求が競合しても、最終的に
    /// フラグが true になり破綻しない（Mutex による直列化の確認）。
    #[test]
    fn concurrent_cancel_requests_are_safe() {
        let reg = Arc::new(CancelRegistry::new());
        let flag = reg.register("job");

        let mut handles = Vec::new();
        for _ in 0..16 {
            let r = Arc::clone(&reg);
            handles.push(thread::spawn(move || {
                r.request_cancel("job");
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert!(flag.load(Ordering::SeqCst));
        assert!(reg.is_cancelled("job"));
    }
}
