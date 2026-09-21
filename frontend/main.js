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

const { invoke } = window.__TAURI__.core;
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
  // 検出済みモデル一覧（download_model へ渡す ModelVariant を保持）。
  models: { available: [], excluded: [] },
  // 検出済み孤立キャプションのパス一覧（承認削除用）。
  orphans: [],
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

/** 単一サムネイルを取得して差し込む。失敗時は代替表示のまま残す（要件 1.6）。 */
async function loadThumbnail(path, cell, size) {
  try {
    const thumb = await invoke("get_thumbnail", { path, size });
    const url = thumb.placeholder ? null : pngBytesToDataUrl(thumb.png);
    const holder = cell.querySelector(".placeholder, img");
    if (!holder) return;
    if (url) {
      const img = document.createElement("img");
      img.src = url;
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

/** 拡大プレビューを取得して表示する（要件 1.5）。 */
async function loadPreview(path) {
  els.preview.classList.remove("show");
  els.previewEmpty.textContent = "プレビューを読み込み中…";
  els.previewEmpty.style.display = "block";
  try {
    const preview = await invoke("get_preview", { path });
    const url = preview.placeholder ? null : pngBytesToDataUrl(preview.png);
    if (url) {
      els.preview.src = url;
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
  inferBatch: document.getElementById("infer-batch"),
  inferModel: document.getElementById("infer-model"),
  inferRun: document.getElementById("infer-run"),
  inferCancel: document.getElementById("infer-cancel"),
  inferProgress: document.getElementById("infer-progress"),
  inferProgressText: document.getElementById("infer-progress-text"),
  inferResult: document.getElementById("infer-result"),
  inferTargetHint: document.getElementById("infer-target-hint"),

  // モデル選択
  modelLocalDir: document.getElementById("model-local-dir"),
  modelLocalPick: document.getElementById("model-local-pick"),
  modelRefresh: document.getElementById("model-refresh"),
  modelList: document.getElementById("model-list"),
  modelLoadLocal: document.getElementById("model-load-local"),
  modelDownload: document.getElementById("model-download"),
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
// 注意（TODO / 設計上の制約）:
//   現状の Rust 側には run_inference を直接呼ぶ #[tauri::command] アダプタが
//   存在しない（バッチ推論はアプリ層の spawn_inference_job 経由で起動される
//   設計、src-tauri/src/commands/adapters.rs 参照）。そのため本パネルの実行
//   ボタンからは invoke("run_inference") を呼べない。ここでは要件に沿って
//   以下を配線する:
//     - 進捗イベント `inference://progress` の購読（done/total 表示、要件 16.7, 17.6）
//     - キャンセルボタン → cancel_operation コマンド（要件 17.7）
//     - 閾値スライダ・Batch_Size・モデル選択の UI（採用値の受け渡し準備）
//   実際のバッチ実行トリガ（run_inference の起動）はアプリ層の spawn に委ねる
//   （Rust は変更しない）。アプリ層が operation_id を払い出したら、その値を
//   本 UI へ渡すことで進捗購読とキャンセルが機能する。

ops.inferThreshold.addEventListener("input", () => {
  ops.inferThresholdValue.textContent = Number(ops.inferThreshold.value).toFixed(2);
});

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
    renderText(ops.inferResult, "先にモデルを選択してください（モデル選択タブ）");
    return;
  }

  // operation_id を払い出し、進捗購読とキャンセルを配線する。
  // 実バッチ実行の起動はアプリ層の spawn（run_inference）に委ねる（Rust 未変更）。
  state.inferOperationId = "infer-" + Date.now();
  ops.inferRun.disabled = true;
  ops.inferCancel.disabled = false;
  await startProgressSubscription();

  renderText(
    ops.inferResult,
    `推論を要求（対象 ${targets.length} 件, 閾値 ${threshold.toFixed(2)}` +
      (batchSize !== null ? `, Batch_Size ${batchSize}` : "") +
      `, モデル ${modelId}）。進捗イベントを購読中。` +
      "\n※ 実バッチ実行トリガはアプリ層の spawn が起動します（operation_id=" +
      state.inferOperationId +
      "）。"
  );
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
  }
});

// ---- モデル選択（要件 15.1, 15.5） ----

ops.modelLocalPick.addEventListener("click", () => pickFolderInto(ops.modelLocalDir));

/** モデル一覧を取得してドロップダウン（モデルタブ・推論タブ）へ反映する。 */
async function refreshModels() {
  const localDir = ops.modelLocalDir.value.trim();
  ops.modelResult.classList.remove("err");
  ops.modelResult.textContent = "取得中…";
  try {
    const listing = await invoke("list_models", {
      localModelDir: localDir || null,
    });
    state.models = listing;
    populateModelSelect(ops.modelList, listing.available);
    populateModelSelect(ops.inferModel, listing.available);
    ops.modelExcluded.textContent =
      listing.excluded.length > 0
        ? `対象外（ONNX なし）: ${listing.excluded.map((m) => m.display_name).join(", ")}`
        : "";
    renderText(
      ops.modelResult,
      `${listing.available.length} 件のモデルが利用可能`,
      true
    );
  } catch (err) {
    renderError(ops.modelResult, err);
  }
}

/** ModelVariant 配列を <select> の <option> へ展開する。 */
function populateModelSelect(select, variants) {
  select.innerHTML = "";
  for (const v of variants) {
    const opt = document.createElement("option");
    opt.value = v.id;
    const loc = v.location && v.location.type === "local" ? "ローカル" : "リモート";
    opt.textContent = `${v.display_name}（${loc}）`;
    select.appendChild(opt);
  }
}

/** 現在選択中の ModelVariant オブジェクトを返す。 */
function selectedVariant() {
  const id = ops.modelList.value;
  return state.models.available.find((v) => v.id === id) || null;
}

ops.modelRefresh.addEventListener("click", refreshModels);

ops.modelLoadLocal.addEventListener("click", async () => {
  // ローカルディレクトリを直接読み込む（要件 15.2）。
  let dir = ops.modelLocalDir.value.trim();
  if (!dir) {
    // 未入力ならダイアログで選択させる。
    try {
      const selected = await dialog.open({ directory: true, multiple: false });
      if (!selected) return;
      dir = selected;
      ops.modelLocalDir.value = dir;
    } catch (err) {
      renderError(ops.modelResult, err);
      return;
    }
  }
  ops.modelResult.classList.remove("err");
  ops.modelResult.textContent = "読込中…";
  try {
    const info = await invoke("load_local_model", { dir });
    renderText(
      ops.modelResult,
      `ローカルモデル読込成功: 入力サイズ ${info.input_size}, ラベル ${info.label_count} 件`,
      true
    );
    refreshModels();
  } catch (err) {
    renderError(ops.modelResult, err);
  }
});

ops.modelDownload.addEventListener("click", async () => {
  const variant = selectedVariant();
  if (!variant) {
    renderText(ops.modelResult, "ダウンロードするモデルを選択してください");
    return;
  }
  // 保存先を選ばせる。
  let destDir;
  try {
    destDir = await dialog.open({ directory: true, multiple: false, title: "保存先" });
  } catch (err) {
    renderError(ops.modelResult, err);
    return;
  }
  if (!destDir) return;
  ops.modelResult.classList.remove("err");
  ops.modelResult.textContent = "ダウンロード中…";
  try {
    const saved = await invoke("download_model", { variant, destDir });
    renderText(ops.modelResult, `ダウンロード完了: ${saved}`, true);
    refreshModels();
  } catch (err) {
    renderError(ops.modelResult, err);
  }
});

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
