// TagEditor フロントエンド（ビルド不要の素の JavaScript）。
//
// Tauri v2 のグローバル API（withGlobalTauri=true）を用いて Rust コマンドを
// 呼び出す。npm ビルドや外部フレームワークを使わず、OS の WebView 上で直接
// 動作させることで軽量化する（要件 16）。
//
// 本ファイルが担う画面（タスク 21.1）:
// - フォルダ選択（要件 1.3）
// - サムネイル一覧とサイズ変更（要件 1.3, 1.4）
// - 拡大プレビュー（要件 1.5）
// - タグ編集領域と保存（要件 2.5）
// - 画像なし / フォルダ読取不可のメッセージ表示（要件 1.7, 1.8）
// - 保存成功 / 失敗表示（要件 2.5）

const { invoke, convertFileSrc } = window.__TAURI__.core;
const dialog = window.__TAURI__.dialog;
// 進捗イベント購読に使用（推論/バッチパネル、要件 16.7, 17.6）。
const tauriEvent = window.__TAURI__.event;

// ---- DOM 参照 ----
const els = {
  selectFolder: document.getElementById("select-folder"),
  folderPath: document.getElementById("folder-path"),
  thumbSize: document.getElementById("thumb-size"),
  thumbSizeValue: document.getElementById("thumb-size-value"),
  status: document.getElementById("status-bar"),
  grid: document.getElementById("thumb-grid"),
  preview: document.getElementById("preview"),
  previewEmpty: document.getElementById("preview-empty"),
  tagText: document.getElementById("tag-text"),
  currentImage: document.getElementById("current-image"),
  saveTags: document.getElementById("save-tags"),
  saveStatus: document.getElementById("save-status"),
};

// ---- アプリ状態 ----
const state = {
  folder: null,
  // 現在フォルダの ImageEntry 一覧。
  items: [],
  // 選択中 Image_File の絶対パス。
  selectedPath: null,
  // 選択モード（true でサムネイルにチェックボックスを表示、タスク 21.2）。
  selectMode: false,
  // 一括操作/推論の対象として選択された Image_File 絶対パスの集合。
  picked: new Set(),
  // capabilities（起動時に取得、要件 13.2, 16.6）。
  capabilities: { windows_only: false },
  // 進行中の推論 operation_id とイベント購読解除関数（キャンセル用）。
  inferOperationId: null,
  inferUnlisten: null,
  // カタログ一覧（list_catalog の CatalogListing を保持）。
  // variants: VariantPresence[]（{ variant: ModelVariant, present: bool }）
  // excluded: ModelVariant[]（必須フィールド欠落で除外、要件 1.4）
  // model_dir_present: Model_Dir が存在するか（要件 3.5）
  models: { variants: [], excluded: [], model_dir_present: false },
  // 進行中のバリアントダウンロード operation_id（variant_id をキーに保持）。
  downloadOps: {},
  // 検出済み孤立キャプションのパス一覧（承認削除用）。
  orphans: [],
  // 直近バッチ推論の Tag_Overview（inference://complete で受信、要件 11.1）。
  // { adopted: TagStat[], discarded: TagStat[] } または null。
  overview: null,
  // Tag_Overview の検索絞り込み後の表示（overview_search の戻り、要件 11.3）。
  // null なら overview を全件表示する。
  overviewFiltered: null,
  // 直近バッチ推論の対象 Image_File パス列（再推論で同一バッチへ再適用、要件 11.6）。
  lastInferTargets: [],
  // 直近バッチ推論の completion イベント購読解除関数。
  completeUnlisten: null,
};

// ---- ユーティリティ ----

/** 上部の状態メッセージを表示する（error=true でエラー配色）。空文字で非表示。 */
function setStatus(message, isError = false) {
  els.status.textContent = message || "";
  els.status.classList.toggle("show", !!message);
  els.status.classList.toggle("error", !!isError && !!message);
}

/** 保存結果メッセージを表示する（要件 2.5）。 */
function setSaveStatus(message, kind) {
  els.saveStatus.textContent = message || "";
  els.saveStatus.classList.remove("ok", "err");
  if (kind) els.saveStatus.classList.add(kind);
}

/**
 * Rust から返る PNG バイト列（Vec<u8> → JSON 配列）を data URL へ変換する。
 * placeholder（生成失敗）時は空配列が返るため null を返す。
 */
function pngBytesToDataUrl(bytes) {
  if (!bytes || bytes.length === 0) return null;
  // 大きい配列でもスタック超過しないようチャンクで文字列化する。
  let binary = "";
  const chunk = 0x8000;
  const arr = bytes instanceof Uint8Array ? bytes : Uint8Array.from(bytes);
  for (let i = 0; i < arr.length; i += chunk) {
    binary += String.fromCharCode.apply(null, arr.subarray(i, i + chunk));
  }
  return "data:image/png;base64," + btoa(binary);
}

/** エラーオブジェクト（AppError DTO or 文字列）を人間可読なメッセージへ整形する。 */
function formatError(err) {
  if (err == null) return "不明なエラー";
  if (typeof err === "string") return err;
  // AppError は { kind, message, path? } 形状で返る。
  if (err.message) return err.message;
  try {
    return JSON.stringify(err);
  } catch {
    return String(err);
  }
}

// ---- サムネイルサイズ（要件 1.4: 64〜512 にクランプ） ----

/** 現在のスライダ値を CSS 変数へ反映し、表示ラベルを更新する。 */
function applyThumbSize() {
  const size = Number(els.thumbSize.value);
  els.thumbSizeValue.textContent = String(size);
  els.grid.style.setProperty("--thumb", size + "px");
  return size;
}

// ---- フォルダ選択と一覧読み込み ----

els.selectFolder.addEventListener("click", async () => {
  try {
    const selected = await dialog.open({
      directory: true,
      multiple: false,
      title: "画像フォルダを選択",
    });
    if (!selected) return; // キャンセル
    await loadFolder(selected);
  } catch (err) {
    setStatus("フォルダ選択に失敗: " + formatError(err), true);
  }
});

/**
 * 指定フォルダの画像を列挙して一覧を描画する。
 * 読取不可はエラーメッセージ（要件 1.8）、空は「画像なし」メッセージ（要件 1.7）。
 */
async function loadFolder(folder) {
  state.folder = folder;
  state.selectedPath = null;
  state.picked.clear();
  updateSelectCount();
  els.folderPath.textContent = folder;
  els.folderPath.title = folder;
  clearDetail();
  els.grid.innerHTML = "";
  setStatus("読み込み中…");

  let listing;
  try {
    listing = await invoke("list_images", { folder });
  } catch (err) {
    // フォルダ読取不可（要件 1.8）。
    setStatus("フォルダを読み取れません: " + formatError(err), true);
    state.items = [];
    return;
  }

  state.items = listing.items || [];

  if (state.items.length === 0) {
    // 画像なし（要件 1.7）。
    setStatus("このフォルダに対応画像がありません", false);
    return;
  }

  let msg = state.items.length + " 件の画像";
  if (listing.truncated) {
    msg += `（全 ${listing.total} 件中、先頭 ${state.items.length} 件を表示）`;
  }
  setStatus(msg, false);

  renderGrid();
}

/** サムネイルグリッドを構築し、各要素へサムネイルを非同期ロードする。 */
function renderGrid() {
  const size = applyThumbSize();
  els.grid.innerHTML = "";

  for (const item of state.items) {
    const cell = document.createElement("div");
    cell.className = "thumb";
    cell.dataset.path = item.path;
    if (state.picked.has(item.path)) cell.classList.add("picked");

    // 選択モード用チェックボックス（body.select-mode 時のみ表示、タスク 21.2）。
    const pick = document.createElement("input");
    pick.type = "checkbox";
    pick.className = "pick";
    pick.checked = state.picked.has(item.path);
    pick.title = "一括操作/推論の対象に含める";
    pick.addEventListener("click", (ev) => ev.stopPropagation());
    pick.addEventListener("change", () => {
      if (pick.checked) state.picked.add(item.path);
      else state.picked.delete(item.path);
      cell.classList.toggle("picked", pick.checked);
      updateSelectCount();
    });
    cell.appendChild(pick);

    const imgHolder = document.createElement("div");
    imgHolder.className = "placeholder";
    imgHolder.textContent = "…";
    cell.appendChild(imgHolder);

    if (item.has_tag_file) {
      const dot = document.createElement("span");
      dot.className = "has-tag";
      dot.title = "タグあり";
      cell.appendChild(dot);
    }

    const name = document.createElement("div");
    name.className = "name";
    name.textContent = item.file_name;
    name.title = item.file_name;
    cell.appendChild(name);

    cell.addEventListener("click", () => selectImage(item.path, cell));
    els.grid.appendChild(cell);

    // サムネイルを非同期ロード（1 件失敗しても他へ波及しない、要件 1.6）。
    loadThumbnail(item.path, cell, size);
  }
}

/**
 * 単一サムネイルを取得して差し込む。失敗時は代替表示のまま残す（要件 1.6）。
 * サムネイルは軽量転送経路（要件 2.10, 2.11, 2.12）を用い、Rust 側がキャッシュへ
 * 書き出したファイルパスを `convertFileSrc`（asset protocol）経由で `<img src>`
 * に直接割り当てる（JSON 数値配列・手動 Base64 化は経ない）。
 */
async function loadThumbnail(path, cell, size) {
  try {
    const result = await invoke("get_thumbnail_path", { path, size });
    const holder = cell.querySelector(".placeholder, img");
    if (!holder) return;
    if (!result.placeholder) {
      const img = document.createElement("img");
      img.src = convertFileSrc(result.path);
      img.alt = "";
      holder.replaceWith(img);
    } else {
      holder.textContent = "表示不可";
    }
  } catch {
    const holder = cell.querySelector(".placeholder, img");
    if (holder && holder.classList.contains("placeholder")) {
      holder.textContent = "表示不可";
    }
  }
}

// ---- 画像選択: プレビュー（要件 1.5）とタグ読み込み ----

async function selectImage(path, cell) {
  state.selectedPath = path;

  // 選択ハイライト。
  for (const el of els.grid.querySelectorAll(".thumb.selected")) {
    el.classList.remove("selected");
  }
  if (cell) cell.classList.add("selected");

  setSaveStatus("");
  els.currentImage.textContent = path;
  els.currentImage.title = path;

  // プレビューとタグを並行取得。
  loadPreview(path);
  loadTags(path);
}

/**
 * 拡大プレビューを取得して表示する（要件 1.5）。
 * 軽量転送経路（要件 2.10, 2.11, 2.12）を用い、`convertFileSrc` でファイル
 * パスを直接 `<img src>` に割り当てる（JSON 数値配列・手動 Base64 化は経ない）。
 */
async function loadPreview(path) {
  els.preview.classList.remove("show");
  els.previewEmpty.textContent = "プレビューを読み込み中…";
  els.previewEmpty.style.display = "block";
  try {
    const result = await invoke("get_preview_path", { path });
    if (!result.placeholder) {
      els.preview.src = convertFileSrc(result.path);
      els.preview.classList.add("show");
      els.previewEmpty.style.display = "none";
    } else {
      els.preview.classList.remove("show");
      els.previewEmpty.textContent = "この画像はプレビューできません";
      els.previewEmpty.style.display = "block";
    }
  } catch (err) {
    els.preview.classList.remove("show");
    els.previewEmpty.textContent = "プレビュー取得に失敗: " + formatError(err);
    els.previewEmpty.style.display = "block";
  }
}

/** 選択画像に対応する Tag_File を読み込み、編集領域へ載せる（要件 2.1）。 */
async function loadTags(path) {
  els.tagText.disabled = true;
  els.saveTags.disabled = true;
  els.tagText.value = "読み込み中…";
  try {
    const result = await invoke("read_tag_file", { imagePath: path });
    els.tagText.value = result.content || "";
    els.tagText.disabled = false;
    els.saveTags.disabled = false;
    els.tagText.placeholder = result.exists
      ? ""
      : "Tag_File はまだありません。保存すると新規作成されます";
  } catch (err) {
    els.tagText.value = "";
    els.tagText.disabled = true;
    els.saveTags.disabled = true;
    setSaveStatus("タグ読み込みに失敗: " + formatError(err), "err");
  }
}

/** 編集内容を Tag_File へ書き込む。成功/失敗を表示する（要件 2.5）。 */
els.saveTags.addEventListener("click", async () => {
  if (!state.selectedPath) return;
  const content = els.tagText.value;
  els.saveTags.disabled = true;
  setSaveStatus("保存中…");
  try {
    await invoke("write_tag_file", {
      imagePath: state.selectedPath,
      content,
    });
    setSaveStatus("保存しました", "ok");
    // タグ有無ドットを更新（このパスのセルに反映）。
    markTagPresence(state.selectedPath, content.trim().length > 0);
  } catch (err) {
    // 失敗時、元ファイルは Rust 側で保持される（要件 2.6）。
    setSaveStatus("保存に失敗: " + formatError(err), "err");
  } finally {
    els.saveTags.disabled = false;
  }
});

/** 指定パスのサムネイルセルのタグ有無インジケータを更新する。 */
function markTagPresence(path, hasTag) {
  const cell = els.grid.querySelector(`.thumb[data-path="${cssEscape(path)}"]`);
  if (!cell) return;
  let dot = cell.querySelector(".has-tag");
  if (hasTag && !dot) {
    dot = document.createElement("span");
    dot.className = "has-tag";
    dot.title = "タグあり";
    cell.appendChild(dot);
  } else if (!hasTag && dot) {
    dot.remove();
  }
}

/** 属性セレクタ用に最小限のエスケープを行う（パスに含まれる \ " をエスケープ）。 */
function cssEscape(value) {
  return value.replace(/\\/g, "\\\\").replace(/"/g, '\\"');
}

/** プレビュー・タグ編集領域を初期状態へ戻す。 */
function clearDetail() {
  state.selectedPath = null;
  els.preview.classList.remove("show");
  els.preview.removeAttribute("src");
  els.previewEmpty.textContent = "画像を選択するとプレビューが表示される";
  els.previewEmpty.style.display = "block";
  els.currentImage.textContent = "";
  els.tagText.value = "";
  els.tagText.disabled = true;
  els.saveTags.disabled = true;
  setSaveStatus("");
}

// ---- サイズスライダ ----

els.thumbSize.addEventListener("input", () => {
  const size = applyThumbSize();
  // グリッド上のサムネイル画像の表示サイズは CSS 変数で追随する。
  // 高解像度化のため一定閾を跨いだら再取得する余地はあるが、
  // ここでは CSS スケールで軽量に追随する（再列挙は行わない）。
  void size;
});

// 初期化。
applyThumbSize();
clearDetail();
setStatus("フォルダを選択してください");

// ===========================================================================
// タスク 21.2: 各操作パネルの結線
//
// 既存の一覧/プレビュー/タグ編集基盤（上記）へ追加する形で、下部の操作パネル
// （タブ切替）を配線する。すべて window.__TAURI__ 経由で登録済み Rust コマンド
// （src-tauri/src/app.rs run() の invoke_handler）を呼び出す。結果は
// OperationReport 形式（succeeded/conflicted/skipped + messages）を一貫して表示。
// ===========================================================================

// ---- 追加 DOM 参照 ----
const ops = {
  // 選択モード（toolbar）
  selectMode: document.getElementById("select-mode"),
  selectAll: document.getElementById("select-all"),
  selectNone: document.getElementById("select-none"),
  selectCount: document.getElementById("select-count"),

  // 一括操作
  bulkTags: document.getElementById("bulk-tags"),
  bulkAdd: document.getElementById("bulk-add"),
  bulkRemove: document.getElementById("bulk-remove"),
  bulkDedup: document.getElementById("bulk-dedup"),
  bulkResult: document.getElementById("bulk-result"),
  bulkTargetHint: document.getElementById("bulk-target-hint"),

  // 集計/フィルタ
  aggRun: document.getElementById("agg-run"),
  aggSummary: document.getElementById("agg-summary"),
  aggResult: document.getElementById("agg-result"),
  filterInclude: document.getElementById("filter-include"),
  filterExclude: document.getElementById("filter-exclude"),
  filterRun: document.getElementById("filter-run"),
  filterClear: document.getElementById("filter-clear"),
  filterResult: document.getElementById("filter-result"),

  // 仕訳
  sortOp: document.getElementById("sort-op"),
  sortSource: document.getElementById("sort-source"),
  sortSourcePick: document.getElementById("sort-source-pick"),
  sortDest: document.getElementById("sort-dest"),
  sortDestPick: document.getElementById("sort-dest-pick"),
  sortRun: document.getElementById("sort-run"),
  sortResult: document.getElementById("sort-result"),

  // サイズ振分
  sizeSource: document.getElementById("size-source"),
  sizeSourcePick: document.getElementById("size-source-pick"),
  sizeThreshold: document.getElementById("size-threshold"),
  sizeDest: document.getElementById("size-dest"),
  sizeDestPick: document.getElementById("size-dest-pick"),
  sizeRun: document.getElementById("size-run"),
  sizeResult: document.getElementById("size-result"),

  // タグ振分
  tagsortSource: document.getElementById("tagsort-source"),
  tagsortSourcePick: document.getElementById("tagsort-source-pick"),
  tagsortJudge: document.getElementById("tagsort-judge"),
  tagsortContains: document.getElementById("tagsort-contains"),
  tagsortContainsPick: document.getElementById("tagsort-contains-pick"),
  tagsortNot: document.getElementById("tagsort-not"),
  tagsortNotPick: document.getElementById("tagsort-not-pick"),
  tagsortRun: document.getElementById("tagsort-run"),
  tagsortResult: document.getElementById("tagsort-result"),

  // 改名/連番/正規化
  renamePattern: document.getElementById("rename-pattern"),
  renameReplacement: document.getElementById("rename-replacement"),
  renameNumberingOn: document.getElementById("rename-numbering-on"),
  renameNumStart: document.getElementById("rename-num-start"),
  renameNumWidth: document.getElementById("rename-num-width"),
  renameRun: document.getElementById("rename-run"),
  renameResult: document.getElementById("rename-result"),
  normalizeRun: document.getElementById("normalize-run"),
  normalizeResult: document.getElementById("normalize-result"),

  // 孤立削除
  orphanFind: document.getElementById("orphan-find"),
  orphanSummary: document.getElementById("orphan-summary"),
  orphanList: document.getElementById("orphan-list"),
  orphanDelete: document.getElementById("orphan-delete"),
  orphanResult: document.getElementById("orphan-result"),

  // 推論/バッチ
  inferThreshold: document.getElementById("infer-threshold"),
  inferThresholdValue: document.getElementById("infer-threshold-value"),
  // Tag_Filter 入力（タスク 17.1、要件 9.1/9.7）。confidence_threshold は
  // inferThreshold を流用するため専用要素は持たない。
  filterKeepTags: document.getElementById("filter-keep-tags"),
  filterExcludeRules: document.getElementById("filter-exclude-rules"),
  filterReplaceRules: document.getElementById("filter-replace-rules"),
  filterAdditional: document.getElementById("filter-additional"),
  filterFraction: document.getElementById("filter-fraction"),
  filterFractionValue: document.getElementById("filter-fraction-value"),
  inferBatch: document.getElementById("infer-batch"),
  inferModel: document.getElementById("infer-model"),
  inferRun: document.getElementById("infer-run"),
  inferCancel: document.getElementById("infer-cancel"),
  inferProgress: document.getElementById("infer-progress"),
  inferProgressText: document.getElementById("infer-progress-text"),
  inferResult: document.getElementById("infer-result"),
  inferTargetHint: document.getElementById("infer-target-hint"),

  // Tag_Overview（バッチ後タグ一覧、要件 11.1〜11.6、タスク 18.1）
  overviewPanel: document.getElementById("overview-panel"),
  overviewSearch: document.getElementById("overview-search"),
  overviewKeep: document.getElementById("overview-keep"),
  overviewExclude: document.getElementById("overview-exclude"),
  overviewRerun: document.getElementById("overview-rerun"),
  overviewAdopted: document.getElementById("overview-adopted"),
  overviewDiscarded: document.getElementById("overview-discarded"),
  overviewAdoptedCount: document.getElementById("overview-adopted-count"),
  overviewDiscardedCount: document.getElementById("overview-discarded-count"),
  overviewResult: document.getElementById("overview-result"),

  // モデル管理（カタログ一覧・存在状態・DL）
  modelRefresh: document.getElementById("model-refresh"),
  modelDirNote: document.getElementById("model-dir-note"),
  modelCatalog: document.getElementById("model-catalog"),
  modelExcluded: document.getElementById("model-excluded"),
  modelResult: document.getElementById("model-result"),

  // Windows 限定
  windowsTab: document.querySelector('.ops-tab[data-tab="windows"]'),
  symlinkTarget: document.getElementById("symlink-target"),
  symlinkPath: document.getElementById("symlink-path"),
  symlinkRun: document.getElementById("symlink-run"),
  symlinkResult: document.getElementById("symlink-result"),
  convertPath: document.getElementById("convert-path"),
  convertDirection: document.getElementById("convert-direction"),
  convertRun: document.getElementById("convert-run"),
  convertResult: document.getElementById("convert-result"),
};

// ---- 共通ユーティリティ ----

/** カンマ区切り文字列を、トリム済み・空要素除去したタグ配列へ変換する。 */
function parseTagList(raw) {
  return (raw || "")
    .split(",")
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

/**
 * OperationReport（succeeded/conflicted/skipped/messages）を結果領域へ描画する。
 * すべての操作パネルで一貫した表示にするための共通ヘルパ。
 */
function renderReport(el, report) {
  el.classList.remove("err");
  el.innerHTML = "";
  const counts = document.createElement("div");
  counts.className = "counts";
  counts.textContent = `成功 ${report.succeeded} / 衝突 ${report.conflicted} / スキップ ${report.skipped}`;
  el.appendChild(counts);
  if (report.messages && report.messages.length > 0) {
    const ul = document.createElement("ul");
    ul.className = "messages";
    for (const m of report.messages) {
      const li = document.createElement("li");
      li.textContent = m;
      ul.appendChild(li);
    }
    el.appendChild(ul);
  }
}

/** 結果領域へエラーメッセージを表示する（既存 formatError を利用）。 */
function renderError(el, err) {
  el.classList.add("err");
  el.textContent = "エラー: " + formatError(err);
}

/** 結果領域へ単純なテキスト（成功系）を表示する。 */
function renderText(el, text, ok = false) {
  el.classList.remove("err");
  el.classList.toggle("ok", !!ok);
  el.textContent = text;
}

/** フォルダ選択ダイアログを開き、選択パスを入力欄へ入れる（未選択は無視）。 */
async function pickFolderInto(input) {
  try {
    const selected = await dialog.open({ directory: true, multiple: false });
    if (selected) input.value = selected;
  } catch (err) {
    setStatus("フォルダ選択に失敗: " + formatError(err), true);
  }
}

/**
 * 一括操作/推論の対象 Image_File パス列を返す（要件 3.1 の対象選択）。
 * targetName は radio group 名。"selected" が選ばれていれば選択集合、
 * それ以外は一覧の全画像を対象にする。
 */
function resolveTargets(targetName) {
  const mode = document.querySelector(
    `input[name="${targetName}"]:checked`
  );
  if (mode && mode.value === "selected") {
    // 選択集合のうち、現在の一覧に含まれるものだけを対象にする。
    const listed = new Set(state.items.map((i) => i.path));
    return [...state.picked].filter((p) => listed.has(p));
  }
  return state.items.map((i) => i.path);
}

/** 選択件数表示と全選択/解除ボタンの活性を更新する。 */
function updateSelectCount() {
  const n = state.picked.size;
  ops.selectCount.textContent = n > 0 ? `選択 ${n} 件` : "";
  const hasItems = state.items.length > 0;
  ops.selectAll.disabled = !hasItems || !state.selectMode;
  ops.selectNone.disabled = n === 0;
  const hint = state.selectMode
    ? `（選択 ${n} 件 / 一覧 ${state.items.length} 件）`
    : "（選択モードOFF: 一覧全件が対象）";
  if (ops.bulkTargetHint) ops.bulkTargetHint.textContent = hint;
  if (ops.inferTargetHint) ops.inferTargetHint.textContent = hint;
}

// ---- 選択モード ----

ops.selectMode.addEventListener("change", () => {
  state.selectMode = ops.selectMode.checked;
  document.body.classList.toggle("select-mode", state.selectMode);
  updateSelectCount();
});

ops.selectAll.addEventListener("click", () => {
  for (const item of state.items) state.picked.add(item.path);
  syncPickCheckboxes();
  updateSelectCount();
});

ops.selectNone.addEventListener("click", () => {
  state.picked.clear();
  syncPickCheckboxes();
  updateSelectCount();
});

/** グリッド上のチェックボックスと picked 集合の表示を同期する。 */
function syncPickCheckboxes() {
  for (const cell of els.grid.querySelectorAll(".thumb")) {
    const path = cell.dataset.path;
    const on = state.picked.has(path);
    const cb = cell.querySelector(".pick");
    if (cb) cb.checked = on;
    cell.classList.toggle("picked", on);
  }
}

// ---- タブ切替 ----

for (const tab of document.querySelectorAll(".ops-tab")) {
  tab.addEventListener("click", () => {
    const name = tab.dataset.tab;
    for (const t of document.querySelectorAll(".ops-tab")) {
      t.classList.toggle("active", t === tab);
    }
    for (const p of document.querySelectorAll(".ops-panel")) {
      p.classList.toggle("active", p.dataset.panel === name);
    }
  });
}

// ---- 一括操作（要件 3.1, 3.3, 3.4） ----

async function runBulk(command, needTags) {
  const targets = resolveTargets("bulk-target");
  if (targets.length === 0) {
    renderText(ops.bulkResult, "対象の画像がありません（フォルダ未選択または選択0件）");
    return;
  }
  const args = { targets };
  if (needTags) {
    const tags = parseTagList(ops.bulkTags.value);
    if (tags.length === 0) {
      renderText(ops.bulkResult, "タグを入力してください");
      return;
    }
    args.tags = tags;
  }
  try {
    const report = await invoke(command, args);
    renderReport(ops.bulkResult, report);
    // タグ変更後、現在選択中の画像なら編集領域を再読込。
    if (state.selectedPath && targets.includes(state.selectedPath)) {
      loadTags(state.selectedPath);
    }
  } catch (err) {
    renderError(ops.bulkResult, err);
  }
}

ops.bulkAdd.addEventListener("click", () => runBulk("bulk_add_tags", true));
ops.bulkRemove.addEventListener("click", () => runBulk("bulk_remove_tags", true));
ops.bulkDedup.addEventListener("click", () => runBulk("dedup_tags", false));

// ---- 集計/フィルタ（要件 6.1〜6.8） ----

ops.aggRun.addEventListener("click", async () => {
  if (!state.folder) {
    renderText(ops.aggResult, "先にフォルダを選択してください");
    return;
  }
  ops.aggResult.classList.remove("err");
  ops.aggResult.textContent = "集計中…";
  try {
    const agg = await invoke("aggregate_tags", { folder: state.folder });
    ops.aggSummary.textContent =
      `${agg.tags.length} 種類のタグ` +
      (agg.unreadable > 0 ? `（読込失敗 ${agg.unreadable} 件を除外）` : "");
    ops.aggResult.innerHTML = "";
    if (agg.tags.length === 0) {
      ops.aggResult.textContent = "有効なタグがありません";
      return;
    }
    const table = document.createElement("table");
    for (const tc of agg.tags) {
      const tr = document.createElement("tr");
      const td1 = document.createElement("td");
      td1.textContent = tc.tag;
      const td2 = document.createElement("td");
      td2.className = "count";
      td2.textContent = String(tc.count);
      tr.appendChild(td1);
      tr.appendChild(td2);
      table.appendChild(tr);
    }
    ops.aggResult.appendChild(table);
  } catch (err) {
    renderError(ops.aggResult, err);
  }
});

ops.filterRun.addEventListener("click", async () => {
  if (!state.folder) {
    renderText(ops.filterResult, "先にフォルダを選択してください");
    return;
  }
  const include = parseTagList(ops.filterInclude.value);
  const exclude = parseTagList(ops.filterExclude.value);
  try {
    const filtered = await invoke("filter_images", {
      folder: state.folder,
      include,
      exclude,
    });
    // フィルタ結果を一覧グリッドへ反映する（要件 6.5〜6.8）。
    state.items = filtered || [];
    state.picked.clear();
    renderGrid();
    updateSelectCount();
    renderText(
      ops.filterResult,
      `フィルタ結果: ${state.items.length} 件を表示`,
      true
    );
  } catch (err) {
    renderError(ops.filterResult, err);
  }
});

ops.filterClear.addEventListener("click", () => {
  ops.filterInclude.value = "";
  ops.filterExclude.value = "";
  if (state.folder) {
    // フォルダ全件を再読込して一覧を復元する。
    loadFolder(state.folder);
    renderText(ops.filterResult, "フィルタを解除しました");
  }
});

// ---- 仕訳/サイズ/タグ振分（要件 7, 9, 10） ----

ops.sortSourcePick.addEventListener("click", () => pickFolderInto(ops.sortSource));
ops.sortDestPick.addEventListener("click", () => pickFolderInto(ops.sortDest));
ops.sizeSourcePick.addEventListener("click", () => pickFolderInto(ops.sizeSource));
ops.sizeDestPick.addEventListener("click", () => pickFolderInto(ops.sizeDest));
ops.tagsortSourcePick.addEventListener("click", () => pickFolderInto(ops.tagsortSource));
ops.tagsortContainsPick.addEventListener("click", () => pickFolderInto(ops.tagsortContains));
ops.tagsortNotPick.addEventListener("click", () => pickFolderInto(ops.tagsortNot));

ops.sortRun.addEventListener("click", async () => {
  const source = ops.sortSource.value.trim();
  const dest = ops.sortDest.value.trim();
  if (!source || !dest) {
    renderText(ops.sortResult, "source と dest を指定してください");
    return;
  }
  try {
    const report = await invoke("sort_files", {
      op: ops.sortOp.value,
      source,
      dest,
    });
    renderReport(ops.sortResult, report);
  } catch (err) {
    renderError(ops.sortResult, err);
  }
});

ops.sizeRun.addEventListener("click", async () => {
  const source = ops.sizeSource.value.trim();
  const destRoot = ops.sizeDest.value.trim();
  const threshold = Number(ops.sizeThreshold.value);
  if (!source || !destRoot) {
    renderText(ops.sizeResult, "source と dest_root を指定してください");
    return;
  }
  if (!Number.isInteger(threshold) || threshold < 1 || threshold > 100000) {
    renderText(ops.sizeResult, "閾値は 1〜100000 の整数で指定してください");
    return;
  }
  try {
    const report = await invoke("sort_by_size", {
      source,
      threshold,
      destRoot,
    });
    renderReport(ops.sizeResult, report);
  } catch (err) {
    renderError(ops.sizeResult, err);
  }
});

ops.tagsortRun.addEventListener("click", async () => {
  const source = ops.tagsortSource.value.trim();
  const judgeTags = parseTagList(ops.tagsortJudge.value);
  const containsDest = ops.tagsortContains.value.trim();
  const notContainsDest = ops.tagsortNot.value.trim();
  if (!source || !containsDest || !notContainsDest) {
    renderText(ops.tagsortResult, "source と 2つの宛先を指定してください");
    return;
  }
  if (judgeTags.length === 0) {
    renderText(ops.tagsortResult, "判定タグを入力してください");
    return;
  }
  try {
    const report = await invoke("sort_by_tag", {
      source,
      judgeTags,
      containsDest,
      notContainsDest,
    });
    renderReport(ops.tagsortResult, report);
  } catch (err) {
    renderError(ops.tagsortResult, err);
  }
});

// ---- 改名/連番/正規化（要件 4, 8） ----

ops.renameNumberingOn.addEventListener("change", () => {
  const on = ops.renameNumberingOn.checked;
  ops.renameNumStart.disabled = !on;
  ops.renameNumWidth.disabled = !on;
});

ops.renameRun.addEventListener("click", async () => {
  if (!state.folder) {
    renderText(ops.renameResult, "先にフォルダを選択してください");
    return;
  }
  const pattern = ops.renamePattern.value;
  const replacement = ops.renameReplacement.value;
  if (!pattern) {
    renderText(ops.renameResult, "正規表現パターンを入力してください");
    return;
  }
  let numbering = null;
  if (ops.renameNumberingOn.checked) {
    numbering = {
      start: Number(ops.renameNumStart.value) || 0,
      width: Number(ops.renameNumWidth.value) || 1,
    };
  }
  try {
    const report = await invoke("rename_regex", {
      folder: state.folder,
      pattern,
      replacement,
      numbering,
    });
    renderReport(ops.renameResult, report);
    // 改名でファイル名が変わるため一覧を再読込。
    loadFolder(state.folder);
  } catch (err) {
    renderError(ops.renameResult, err);
  }
});

ops.normalizeRun.addEventListener("click", async () => {
  if (!state.folder) {
    renderText(ops.normalizeResult, "先にフォルダを選択してください");
    return;
  }
  try {
    const report = await invoke("normalize_caption_filenames", {
      folder: state.folder,
    });
    renderReport(ops.normalizeResult, report);
  } catch (err) {
    renderError(ops.normalizeResult, err);
  }
});

// ---- 孤立削除（承認一覧、要件 11.1〜11.4） ----

ops.orphanFind.addEventListener("click", async () => {
  if (!state.folder) {
    renderText(ops.orphanList, "先にフォルダを選択してください");
    return;
  }
  ops.orphanResult.textContent = "";
  ops.orphanList.classList.remove("err");
  ops.orphanList.textContent = "検出中…";
  try {
    const orphans = await invoke("find_orphan_captions", {
      folder: state.folder,
    });
    state.orphans = orphans || [];
    ops.orphanSummary.textContent = `${state.orphans.length} 件の孤立キャプション`;
    ops.orphanList.innerHTML = "";
    if (state.orphans.length === 0) {
      ops.orphanList.textContent = "孤立キャプションはありません";
      ops.orphanDelete.disabled = true;
      return;
    }
    // 承認のため一覧を提示する（要件 11.2）。ユーザーが確認してから削除する。
    const ul = document.createElement("ul");
    for (const p of state.orphans) {
      const li = document.createElement("li");
      li.textContent = p;
      ul.appendChild(li);
    }
    ops.orphanList.appendChild(ul);
    ops.orphanDelete.disabled = false;
  } catch (err) {
    renderError(ops.orphanList, err);
    ops.orphanDelete.disabled = true;
  }
});

ops.orphanDelete.addEventListener("click", async () => {
  if (state.orphans.length === 0) return;
  // 承認確認（要件 11.2）。破壊的操作のため明示同意を取る。
  const ok = window.confirm(
    `${state.orphans.length} 件の孤立キャプションを削除します。よろしいですか？`
  );
  if (!ok) return;
  ops.orphanDelete.disabled = true;
  try {
    const result = await invoke("delete_orphan_captions", {
      paths: state.orphans,
    });
    renderText(ops.orphanResult, `削除しました: ${result.deleted} 件`, true);
    // 一覧をクリアして再検出を促す。
    state.orphans = [];
    ops.orphanList.innerHTML = "";
    ops.orphanSummary.textContent = "";
  } catch (err) {
    renderError(ops.orphanResult, err);
    ops.orphanDelete.disabled = false;
  }
});

// ---- 推論/バッチ（進捗・キャンセル・閾値・Batch_Size、要件 16.7, 17.6, 17.7） ----
//
// バッチ推論の起動は #[tauri::command] start_inference（src-tauri/src/commands/
// adapters.rs）へ委譲する。operation_id 発行・進捗購読開始の後に invoke で
// start_inference を呼び、Rust 側がバックグラウンドスレッドで spawn_inference_job
// を起動する（要件 2.1, 2.2, 2.3）。セッション未ロードなど呼び出し自体が失敗
// した場合は renderError で表示し、ボタン活性化/購読解除を行う（要件 2.6）。

ops.inferThreshold.addEventListener("input", () => {
  ops.inferThresholdValue.textContent = Number(ops.inferThreshold.value).toFixed(2);
});

ops.filterFraction.addEventListener("input", () => {
  ops.filterFractionValue.textContent = Number(ops.filterFraction.value).toFixed(2);
});

/**
 * カンマまたは改行区切りの入力を、トリム済み・空要素除去した文字列配列へ変換する
 * （keep/exclude/additional の共通パーサ）。
 */
function parseCommaOrNewlineList(raw) {
  return (raw || "")
    .split(/[,\n]/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0);
}

/**
 * Replace_Rules 入力（1 行 1 対「検索,置換」）を [検索, 置換][] へパースする。
 * 検索が空の行は無視する。置換は空文字を許容する（タグ削除的な置換）。
 * 検索側に「,」を含めたい用途は想定せず、最初の「,」で検索/置換に分割する。
 */
function parseReplaceRules(raw) {
  const rules = [];
  for (const line of (raw || "").split(/\n/)) {
    const trimmed = line.trim();
    if (trimmed.length === 0) continue;
    const idx = trimmed.indexOf(",");
    if (idx < 0) continue; // 「検索,置換」形式でない行は無視。
    const search = trimmed.slice(0, idx).trim();
    const replacement = trimmed.slice(idx + 1).trim();
    if (search.length === 0) continue;
    rules.push([search, replacement]);
  }
  return rules;
}

/**
 * Tag_Filter 入力要素から RawTagFilter DTO を組み立てる（要件 9.1）。
 * confidence_threshold は推論の閾値スライダを流用し、fraction_threshold は
 * 専用スライダの値（0 で非適用、要件 10.3）を用いる。
 */
function buildRawTagFilter() {
  return {
    keep: parseCommaOrNewlineList(ops.filterKeepTags.value),
    exclude: parseCommaOrNewlineList(ops.filterExcludeRules.value),
    replace: parseReplaceRules(ops.filterReplaceRules.value),
    additional: parseCommaOrNewlineList(ops.filterAdditional.value),
    confidence_threshold: Number(ops.inferThreshold.value),
    fraction_threshold: Number(ops.filterFraction.value),
  };
}

/** 進捗イベント購読を開始する。既存購読があれば解除してから張り直す。 */
async function startProgressSubscription() {
  await stopProgressSubscription();
  ops.inferProgress.value = 0;
  ops.inferProgressText.textContent = "";
  try {
    state.inferUnlisten = await tauriEvent.listen("inference://progress", (evt) => {
      const p = evt.payload || {};
      // 進行中の operation_id が判明していれば、それ以外は無視する。
      if (state.inferOperationId && p.operation_id !== state.inferOperationId) {
        return;
      }
      const total = p.total || 0;
      const done = p.done || 0;
      ops.inferProgress.max = total > 0 ? total : 100;
      ops.inferProgress.value = done;
      ops.inferProgressText.textContent =
        total > 0 ? `処理済み ${done} / ${total}` : "";
      // 完了したら結果件数を表示し、キャンセルを閉じる。
      if (total > 0 && done >= total) {
        renderText(ops.inferResult, `完了: ${done} / ${total} 件を処理`, true);
        finishInference();
      }
    });
  } catch (err) {
    renderError(ops.inferResult, err);
  }
}

/** 進捗イベント購読を解除する。 */
async function stopProgressSubscription() {
  if (state.inferUnlisten) {
    try {
      state.inferUnlisten();
    } catch {
      /* 解除失敗は無視 */
    }
    state.inferUnlisten = null;
  }
}

/**
 * バッチ推論完了イベント（inference://complete）購読を開始する（要件 11.1）。
 * Rust の start_inference はバックグラウンドスレッドで推論するため戻り値では
 * overview を返せない。完了時に InferBatchResult（overview を含む）を emit する
 * ので、それを購読して Tag_Overview を描画する（タスク 18.1）。
 * 既存購読があれば解除してから張り直す。
 */
async function startCompleteSubscription() {
  await stopCompleteSubscription();
  try {
    state.completeUnlisten = await tauriEvent.listen(
      "inference://complete",
      (evt) => {
        const result = evt.payload || {};
        // overview を state に保持して 2 区分描画する（要件 11.1, 11.2）。
        state.overview = result.overview || { adopted: [], discarded: [] };
        state.overviewFiltered = null;
        // 検索ボックスは新バッチで一旦クリアする。
        ops.overviewSearch.value = "";
        renderOverview();
        ops.overviewPanel.hidden = false;
        renderText(
          ops.overviewResult,
          `一覧を更新（採用 ${state.overview.adopted.length} / 不採用 ${state.overview.discarded.length}）`,
          true
        );
      }
    );
  } catch (err) {
    renderError(ops.inferResult, err);
  }
}

/** バッチ推論完了イベント購読を解除する。 */
async function stopCompleteSubscription() {
  if (state.completeUnlisten) {
    try {
      state.completeUnlisten();
    } catch {
      /* 解除失敗は無視 */
    }
    state.completeUnlisten = null;
  }
}

/** 推論の後片付け（ボタン活性・購読解除）。 */
function finishInference() {
  ops.inferRun.disabled = false;
  ops.inferCancel.disabled = true;
  state.inferOperationId = null;
  stopProgressSubscription();
}

ops.inferRun.addEventListener("click", async () => {
  const targets = resolveTargets("infer-target");
  if (targets.length === 0) {
    renderText(ops.inferResult, "対象の画像がありません（フォルダ未選択または選択0件）");
    return;
  }
  const threshold = Number(ops.inferThreshold.value);
  const batchRaw = ops.inferBatch.value.trim();
  const batchSize = batchRaw === "" ? null : Number(batchRaw);
  if (batchSize !== null && (!Number.isInteger(batchSize) || batchSize < 1 || batchSize > 64)) {
    renderText(ops.inferResult, "Batch_Size は 1〜64 の整数で指定してください");
    return;
  }
  const modelId = ops.inferModel.value;
  if (!modelId) {
    renderText(ops.inferResult, "先にモデルを選択してください（モデル管理タブ）");
    return;
  }

  // operation_id を払い出し、進捗購読とキャンセルを配線した後に実推論を起動する。
  state.inferOperationId = "infer-" + Date.now();
  // 再推論（要件 11.6）のため、対象バッチを保持する。
  state.lastInferTargets = targets.slice();
  ops.inferRun.disabled = true;
  ops.inferCancel.disabled = false;
  await startProgressSubscription();
  // 完了イベント（overview）購読を開始する（要件 11.1）。
  await startCompleteSubscription();

  renderText(
    ops.inferResult,
    `推論を要求（対象 ${targets.length} 件, 閾値 ${threshold.toFixed(2)}` +
      (batchSize !== null ? `, Batch_Size ${batchSize}` : "") +
      `, モデル ${modelId}）。進捗イベントを購読中。`
  );

  // Rust の start_inference は variant_id + filter（RawTagFilter）を取る
  // （旧 threshold は廃止）。Tag_Filter 入力 UI（keep/exclude/replace/additional/
  // confidence_threshold/fraction_threshold）から RawTagFilter を組み立てる
  // （タスク 17.1、要件 9.1）。confidence_threshold は閾値スライダを流用する。
  const filter = buildRawTagFilter();

  try {
    await invoke("start_inference", {
      variantId: modelId,
      filter,
      imagePaths: targets,
      batchSize,
      operationId: state.inferOperationId,
    });
  } catch (err) {
    // モデル未ロード、または無効な正規表現パターン（InvalidInput、要件 9.7）等で
    // 起動自体が失敗した場合（要件 2.6）。formatError が AppError.message を
    // 表示するため、無効パターンの詳細もそのまま UI に出る。
    renderError(ops.inferResult, err);
    finishInference();
  }
});

ops.inferCancel.addEventListener("click", async () => {
  if (!state.inferOperationId) return;
  try {
    const requested = await invoke("cancel_operation", {
      operationId: state.inferOperationId,
    });
    renderText(
      ops.inferResult,
      requested
        ? "キャンセルを要求しました"
        : "対象の処理は登録されていません（既に完了/未開始）"
    );
  } catch (err) {
    renderError(ops.inferResult, err);
  } finally {
    finishInference();
    // キャンセル時は完了イベントが来ないため、完了購読も解除する。
    stopCompleteSubscription();
  }
});

// ---- Tag_Overview（バッチ後タグ一覧、要件 11.1〜11.6、タスク 18.1） ----
//
// バッチ推論完了イベント（inference://complete）で受け取った TagOverview を
// 採用/不採用の 2 区分で描画する。検索は overview_search、keep/exclude 送出は
// overview_send_keep / overview_send_exclude、再推論は更新後 filter で同一
// バッチへ start_inference を再実行して完了イベントで一覧を更新する。

/**
 * 現在表示すべき overview（検索絞り込み中なら overviewFiltered、無ければ
 * overview）を返す。overview が無ければ null。
 */
function currentOverview() {
  if (state.overviewFiltered) return state.overviewFiltered;
  return state.overview;
}

/** 1 タグ（TagStat）の行要素を構築する（タグ名＋代表確信度、要件 11.2）。 */
function buildOverviewRow(stat) {
  const row = document.createElement("div");
  row.className = "overview-row";
  const name = document.createElement("span");
  name.className = "overview-tag-name";
  name.textContent = stat.name;
  name.title = stat.name;
  const conf = document.createElement("span");
  conf.className = "overview-tag-conf";
  // representative_confidence は 0.0〜1.0。出現画像数も併記する。
  const pct = (Number(stat.representative_confidence) * 100).toFixed(1);
  conf.textContent = `${pct}%（${stat.image_count} 枚）`;
  row.appendChild(name);
  row.appendChild(conf);
  return row;
}

/** 1 区分の一覧をリスト要素へ描画する。空なら「なし」を表示する。 */
function renderOverviewList(el, stats) {
  el.innerHTML = "";
  if (!stats || stats.length === 0) {
    const empty = document.createElement("p");
    empty.className = "inline-note";
    empty.textContent = "なし";
    el.appendChild(empty);
    return;
  }
  for (const stat of stats) {
    el.appendChild(buildOverviewRow(stat));
  }
}

/** Tag_Overview を採用/不採用の 2 区分で描画する（要件 11.1, 11.2）。 */
function renderOverview() {
  const ov = currentOverview();
  const adopted = (ov && ov.adopted) || [];
  const discarded = (ov && ov.discarded) || [];
  renderOverviewList(ops.overviewAdopted, adopted);
  renderOverviewList(ops.overviewDiscarded, discarded);
  ops.overviewAdoptedCount.textContent = `${adopted.length} 件`;
  ops.overviewDiscardedCount.textContent = `${discarded.length} 件`;
}

/**
 * 現在の表示（検索絞り込み後含む）に含まれる全タグ名を返す。
 * keep/exclude 送出は「表示中のタグ」を対象にする（要件 11.4, 11.5）。
 */
function visibleOverviewTagNames() {
  const ov = currentOverview();
  if (!ov) return [];
  const names = [];
  for (const s of ov.adopted || []) names.push(s.name);
  for (const s of ov.discarded || []) names.push(s.name);
  return names;
}

/** 検索入力で overview_search を呼び表示を絞り込む（要件 11.3）。 */
async function applyOverviewSearch() {
  if (!state.overview) return;
  const query = ops.overviewSearch.value;
  if (!query || query.trim().length === 0) {
    // 空クエリは全件表示（overviewFiltered を解除）。
    state.overviewFiltered = null;
    renderOverview();
    return;
  }
  try {
    // design のコマンド境界に従い overview_search へ委譲する（要件 11.3）。
    const filtered = await invoke("overview_search", {
      overview: state.overview,
      query,
    });
    state.overviewFiltered = filtered;
    renderOverview();
  } catch (err) {
    renderError(ops.overviewResult, err);
  }
}

// 検索入力はデバウンスして overview_search を呼ぶ。
let overviewSearchTimer = null;
ops.overviewSearch.addEventListener("input", () => {
  if (overviewSearchTimer) clearTimeout(overviewSearchTimer);
  overviewSearchTimer = setTimeout(applyOverviewSearch, 200);
});

/**
 * 表示中タグを Keep_Tags / Exclude_Rules へ送る（要件 11.4, 11.5）。
 * command（overview_send_keep / overview_send_exclude）に現在の RawTagFilter と
 * 表示中タグを渡し、更新後 filter を Tag_Filter 入力欄（keep/exclude テキスト
 * エリア）へ反映する。これにより次回推論・再推論に一貫して効く。
 */
async function sendVisibleTags(command, targetTextarea) {
  const tags = visibleOverviewTagNames();
  if (tags.length === 0) {
    renderText(ops.overviewResult, "送出対象のタグがありません（表示中0件）");
    return;
  }
  const filter = buildRawTagFilter();
  try {
    const updated = await invoke(command, { filter, tags });
    // 更新後の keep / exclude を Tag_Filter 入力欄へ反映する（改行区切り）。
    ops.filterKeepTags.value = (updated.keep || []).join("\n");
    ops.filterExcludeRules.value = (updated.exclude || []).join("\n");
    void targetTextarea;
    renderText(
      ops.overviewResult,
      `${tags.length} 件のタグを送出しました。更新後フィルタで再推論すると一覧へ反映されます（要件 11.6）`,
      true
    );
  } catch (err) {
    renderError(ops.overviewResult, err);
  }
}

ops.overviewKeep.addEventListener("click", () =>
  sendVisibleTags("overview_send_keep", ops.filterKeepTags)
);
ops.overviewExclude.addEventListener("click", () =>
  sendVisibleTags("overview_send_exclude", ops.filterExcludeRules)
);

/**
 * 更新後フィルタで同一バッチへ再推論する（要件 11.6）。
 *
 * rerun_inference コマンドは各画像の生 Predicted_Tag 列（per_image_predicted）を
 * 要するが、start_inference の結果（InferBatchResult）は overview のみで生 Tag 列を
 * 含まないため、フロントは per_image_predicted を保持できない。よって再推論は
 * 「保持した直近バッチ（lastInferTargets）へ、更新後 filter で start_inference を
 * 再実行する」形で実現する。完了イベント（inference://complete）で更新後の
 * overview が届き一覧が再描画される。これにより Keep/Exclude 送出後の結果が
 * 一覧へ反映される（要件 11.6）。
 */
ops.overviewRerun.addEventListener("click", async () => {
  if (state.lastInferTargets.length === 0) {
    renderText(ops.overviewResult, "再推論する対象バッチがありません（先に推論を実行してください）");
    return;
  }
  const modelId = ops.inferModel.value;
  if (!modelId) {
    renderText(ops.overviewResult, "先にモデルを選択してください（モデル管理タブ）");
    return;
  }
  const batchRaw = ops.inferBatch.value.trim();
  const batchSize = batchRaw === "" ? null : Number(batchRaw);

  state.inferOperationId = "rerun-" + Date.now();
  ops.inferRun.disabled = true;
  ops.inferCancel.disabled = false;
  await startProgressSubscription();
  await startCompleteSubscription();

  const filter = buildRawTagFilter();
  renderText(ops.overviewResult, "更新後フィルタで再推論中…");
  try {
    await invoke("start_inference", {
      variantId: modelId,
      filter,
      imagePaths: state.lastInferTargets,
      batchSize,
      operationId: state.inferOperationId,
    });
  } catch (err) {
    renderError(ops.overviewResult, err);
    finishInference();
    stopCompleteSubscription();
  }
});

// ---- モデル管理（カタログ一覧・存在状態・ダウンロード、要件 1.1, 1.2, 3.1, 3.5, 5.1, 5.3） ----
//
// 固定 Model_Dir 方針（要件 2）に伴い、任意ローカルディレクトリ選択 UI は撤去した。
// Model_Dir は Rust が PathResolver から解決するため base_dir 引数は渡さない。

/** ModelFamily（serde snake_case）を日本語表示へ変換する。 */
function familyLabel(family) {
  if (family === "wd14") return "WD14";
  if (family === "ml_danbooru") return "ML-Danbooru";
  return family || "";
}

/**
 * カタログを取得して Model_Management_Tab へ描画する（要件 1.1, 1.2, 3.1, 3.5）。
 * 推論タブのモデル選択（inferModel）へも全 variant を反映する。
 */
async function refreshModels() {
  ops.modelResult.classList.remove("err");
  ops.modelResult.textContent = "取得中…";
  try {
    // base_dir は Rust が解決するため引数不要（固定 Model_Dir 方針）。
    const listing = await invoke("list_catalog");
    state.models = {
      variants: listing.variants || [],
      excluded: listing.excluded || [],
      model_dir_present: !!listing.model_dir_present,
    };
    renderCatalog();
    populateInferModelSelect();

    // Model_Dir 未作成の旨を表示（要件 3.5）。
    ops.modelDirNote.textContent = state.models.model_dir_present
      ? ""
      : "Model_Dir は未作成（初回ダウンロード時に作成される）";

    // 除外バリアント（必須フィールド欠落、要件 1.2/1.4）。
    ops.modelExcluded.textContent =
      state.models.excluded.length > 0
        ? `対象外: ${state.models.excluded.map((m) => m.display_name || m.id).join(", ")}`
        : "";

    const presentCount = state.models.variants.filter((vp) => vp.present).length;
    renderText(
      ops.modelResult,
      `カタログ ${state.models.variants.length} 件（取得済み ${presentCount} 件）`,
      true
    );
  } catch (err) {
    renderError(ops.modelResult, err);
  }
}

/**
 * カタログ各バリアントを行として描画する（要件 1.1, 3.1, 5.1）。
 * 各行: 表示名・系統・存在バッジ（Model_Present/Not_Present）・DL ボタン・
 * キャンセルボタン・進捗バー。カタログが空なら登録済みモデルなしを表示（要件 1.2）。
 */
function renderCatalog() {
  ops.modelCatalog.innerHTML = "";
  if (state.models.variants.length === 0) {
    const empty = document.createElement("p");
    empty.className = "inline-note";
    empty.textContent = "登録済みモデルがありません";
    ops.modelCatalog.appendChild(empty);
    return;
  }

  for (const vp of state.models.variants) {
    ops.modelCatalog.appendChild(buildVariantRow(vp));
  }
}

/** 1 バリアント分の行要素を構築する。 */
function buildVariantRow(vp) {
  const variant = vp.variant;
  const row = document.createElement("div");
  row.className = "model-row";
  row.dataset.variantId = variant.id;

  // 表示名 + 系統。
  const info = document.createElement("div");
  info.className = "model-info";
  const name = document.createElement("span");
  name.className = "model-name";
  name.textContent = variant.display_name;
  name.title = variant.id;
  const fam = document.createElement("span");
  fam.className = "model-family";
  fam.textContent = familyLabel(variant.family);
  info.appendChild(name);
  info.appendChild(fam);
  row.appendChild(info);

  // 存在バッジ（要件 3.1）。
  const badge = document.createElement("span");
  badge.className = "model-badge " + (vp.present ? "present" : "not-present");
  badge.textContent = vp.present ? "取得済み" : "未取得";
  row.appendChild(badge);

  // 操作領域（DL / キャンセル）。
  const actions = document.createElement("div");
  actions.className = "model-actions";
  const dlBtn = document.createElement("button");
  dlBtn.type = "button";
  dlBtn.className = "model-download-btn";
  dlBtn.textContent = vp.present ? "再ダウンロード" : "ダウンロード";
  dlBtn.addEventListener("click", () => startVariantDownload(variant.id, row));
  const cancelBtn = document.createElement("button");
  cancelBtn.type = "button";
  cancelBtn.className = "model-cancel-btn danger";
  cancelBtn.textContent = "キャンセル";
  cancelBtn.disabled = true;
  cancelBtn.addEventListener("click", () => cancelVariantDownload(variant.id));
  actions.appendChild(dlBtn);
  actions.appendChild(cancelBtn);
  row.appendChild(actions);

  // 進捗バー（0〜100%、要件 5.3）。
  const progWrap = document.createElement("div");
  progWrap.className = "model-progress-wrap";
  const prog = document.createElement("progress");
  prog.className = "model-progress";
  prog.max = 100;
  prog.value = 0;
  prog.hidden = true;
  const progText = document.createElement("span");
  progText.className = "model-progress-text inline-note";
  progWrap.appendChild(prog);
  progWrap.appendChild(progText);
  row.appendChild(progWrap);

  return row;
}

/** 推論タブのモデル選択に全 variant を反映する（present 問わず選択可）。 */
function populateInferModelSelect() {
  const select = ops.inferModel;
  const prev = select.value;
  select.innerHTML = "";
  for (const vp of state.models.variants) {
    const v = vp.variant;
    const opt = document.createElement("option");
    opt.value = v.id;
    opt.textContent = vp.present
      ? v.display_name
      : `${v.display_name}（未取得・DL後推論）`;
    select.appendChild(opt);
  }
  // 以前の選択を可能なら維持する。
  if (prev && state.models.variants.some((vp) => vp.variant.id === prev)) {
    select.value = prev;
  }
}

ops.modelRefresh.addEventListener("click", refreshModels);

/**
 * バリアントのダウンロードを開始する（要件 5.1, 5.2, 5.3）。
 * spawn_variant_download（variant_id + operation_id）を呼び、進捗イベント
 * （inference://progress）を operation_id でフィルタして 0〜100% 表示する。
 * 完了検知（done>=total）後は refreshModels で present 状態を更新する（要件 5.5）。
 */
async function startVariantDownload(variantId, row) {
  // 既に進行中なら二重起動しない。
  if (state.downloadOps[variantId]) return;

  const dlBtn = row.querySelector(".model-download-btn");
  const cancelBtn = row.querySelector(".model-cancel-btn");
  const prog = row.querySelector(".model-progress");
  const progText = row.querySelector(".model-progress-text");

  const operationId = "download-" + variantId + "-" + Date.now();

  // 進捗イベント購読（operation_id でフィルタ）。
  let unlisten = null;
  const cleanup = () => {
    if (unlisten) {
      try {
        unlisten();
      } catch {
        /* 解除失敗は無視 */
      }
    }
    delete state.downloadOps[variantId];
    dlBtn.disabled = false;
    cancelBtn.disabled = true;
    prog.hidden = true;
  };

  try {
    unlisten = await tauriEvent.listen("inference://progress", (evt) => {
      const p = evt.payload || {};
      if (p.operation_id !== operationId) return;
      const total = p.total || 0;
      const done = p.done || 0;
      const pct = total > 0 ? Math.round((done / total) * 100) : 0;
      prog.value = pct;
      progText.textContent = `${pct}%`;
      if (total > 0 && done >= total) {
        // 完了検知（要件 5.5）。present 状態を再取得で反映する。
        progText.textContent = "完了";
        cleanup();
        refreshModels();
      }
    });
  } catch (err) {
    renderError(ops.modelResult, err);
    return;
  }

  state.downloadOps[variantId] = { operationId, cleanup };
  dlBtn.disabled = true;
  cancelBtn.disabled = false;
  prog.hidden = false;
  prog.value = 0;
  progText.textContent = "0%";

  try {
    // 別スレッドで spawn し即戻る（要件 5.2）。完了検知は進捗イベントで行う。
    await invoke("spawn_variant_download", { variantId, operationId });
  } catch (err) {
    renderError(ops.modelResult, err);
    cleanup();
  }
}

/** 進行中のバリアントダウンロードをキャンセルする（要件 5.4）。 */
async function cancelVariantDownload(variantId) {
  const entry = state.downloadOps[variantId];
  if (!entry) return;
  try {
    await invoke("cancel_operation", { operationId: entry.operationId });
    renderText(ops.modelResult, "キャンセルを要求しました");
  } catch (err) {
    renderError(ops.modelResult, err);
  } finally {
    entry.cleanup();
    // キャンセル後の存在状態を再取得（部分ファイルは Rust が除去、要件 5.4）。
    refreshModels();
  }
}

// ---- Windows 限定（symlink 作成 + パス変換、要件 13.2, 16.6） ----

ops.symlinkRun.addEventListener("click", async () => {
  const linkTarget = ops.symlinkTarget.value.trim();
  const linkPath = ops.symlinkPath.value.trim();
  if (!linkTarget || !linkPath) {
    renderText(ops.symlinkResult, "リンク元と作成先を指定してください");
    return;
  }
  try {
    await invoke("create_symlink", { linkTarget, linkPath });
    renderText(ops.symlinkResult, "シンボリックリンクを作成しました", true);
  } catch (err) {
    renderError(ops.symlinkResult, err);
  }
});

ops.convertRun.addEventListener("click", async () => {
  const path = ops.convertPath.value.trim();
  if (!path) {
    renderText(ops.convertResult, "変換するパスを入力してください");
    return;
  }
  try {
    const converted = await invoke("convert_path", {
      path,
      direction: ops.convertDirection.value,
    });
    renderText(ops.convertResult, "変換結果: " + converted, true);
  } catch (err) {
    renderError(ops.convertResult, err);
  }
});

// ---- 起動時: capabilities による Windows 限定 UI の表示制御（要件 13.2, 16.6） ----

async function initCapabilities() {
  try {
    const caps = await invoke("capabilities");
    state.capabilities = caps;
    // 非 Windows なら Windows_Only_Feature のタブ/パネルを描画しない（要件 13.2, 16.6）。
    if (!caps.windows_only) {
      if (ops.windowsTab) ops.windowsTab.remove();
      const panel = document.querySelector('.ops-panel[data-panel="windows"]');
      if (panel) panel.remove();
    } else {
      if (ops.windowsTab) ops.windowsTab.hidden = false;
    }
  } catch (err) {
    // 取得失敗時は保守的に Windows 限定 UI を隠す。
    if (ops.windowsTab) ops.windowsTab.remove();
    const panel = document.querySelector('.ops-panel[data-panel="windows"]');
    if (panel) panel.remove();
    setStatus("capabilities 取得に失敗: " + formatError(err), true);
  }
}

// 追加初期化。
updateSelectCount();
initCapabilities();
refreshModels();
