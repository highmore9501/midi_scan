// MIDI Manager 前端逻辑：通过 Tauri invoke 调用 Rust 命令。

"use strict";

const invoke = (cmd, args) => window.__TAURI__.core.invoke(cmd, args);

const $ = (sel) => document.querySelector(sel);

const state = {
  page: "scan",
  scanDirs: [],
  instruments: [], // {id, name, checked}
  instFilter: "",
  results: [],
  page: 0,
  pageSize: 50,
  groups: [],
  expandedGroup: null,
  memberChecked: [], // true = 删除
  pendingDelete: null, // {groupId, keepId, deleteIds}
};

// ---------- 状态栏 ----------

function setStatus(text) {
  $("#statusbar").textContent = text;
}

async function run(label, fn) {
  setStatus(label + "……");
  try {
    const r = await fn();
    return r;
  } catch (e) {
    setStatus("失败：" + e);
    console.error(e);
    throw e;
  }
}

// ---------- 页面切换 ----------

function switchPage(name) {
  state.page = name;
  document.querySelectorAll("nav button").forEach((b) => {
    b.classList.toggle("active", b.dataset.page === name);
  });
  document.querySelectorAll(".page").forEach((p) => {
    p.classList.toggle("active", p.id === "page-" + name);
  });
  if (name === "library") loadInstruments();
  if (name === "dedup") refreshGroups();
  if (name === "settings") refreshStats();
}

// ---------- 扫描页 ----------

function renderScanDirs() {
  $("#scanDirList").innerHTML = state.scanDirs
    .map((d, i) => `<div>${i + 1}. ${escapeHtml(d)}</div>`)
    .join("");
}

$("#addDirBtn").addEventListener("click", () => {
  const v = $("#scanDirInput").value.trim();
  if (v) {
    state.scanDirs.push(v);
    $("#scanDirInput").value = "";
    renderScanDirs();
  }
});

$("#browseBtn").addEventListener("click", async () => {
  try {
    const dir = await invoke("pick_folder");
    if (dir) $("#scanDirInput").value = dir;
  } catch (e) {
    setStatus("选择目录失败：" + e);
  }
});

$("#scanDirInput").addEventListener("keydown", (e) => {
  if (e.key === "Enter") $("#addDirBtn").click();
});

$("#clearScanBtn").addEventListener("click", () => {
  state.scanDirs = [];
  renderScanDirs();
  $("#scanStatus").textContent = "";
  $("#scanProgress").textContent = "";
});

// ---------- 扫描进度轮询 ----------

let scanTimer = null;
const SPIN = ["|", "/", "-", "\\"];

function renderScanProgress(p) {
  const el = $("#scanProgress");
  if (p.running) {
    const frame = SPIN[Math.floor(Date.now() / 200) % SPIN.length];
    const head = p.cancelled
      ? `<span class="spin">${frame}</span> 正在停止扫描……`
      : `<span class="spin">${frame}</span> 正在扫描：${escapeHtml(p.current_file || "（收集文件…）")}`;
    const dedupText =
      p.deleted_duplicates > 0
        ? ` · 已自动删除重复 ${p.deleted_duplicates}`
        : "";
    el.innerHTML =
      head +
      "<br/>" +
      `已发现 ${p.found} · 新增 ${p.new} · 更新 ${p.updated} · 跳过 ${p.skipped} · 失败 ${p.failed}${dedupText}`;
  } else {
    el.textContent = "";
  }
}

function stopScanPolling() {
  if (scanTimer !== null) {
    clearInterval(scanTimer);
    scanTimer = null;
  }
}

$("#startScanBtn").addEventListener("click", async () => {
  if (state.scanDirs.length === 0) {
    setStatus("请先添加至少一个扫描目录");
    return;
  }
  const btn = $("#startScanBtn");
  const stopBtn = $("#stopScanBtn");
  btn.disabled = true;
  stopBtn.disabled = false;
  try {
    await invoke("scan", {
      dirs: state.scanDirs,
      autoDedup: $("#autoDedupSelect").value,
    });
    setStatus("扫描中……（可切换页面，点「停止扫描」可随时停止）");
    $("#scanStatus").textContent = "";
    stopScanPolling();
    scanTimer = setInterval(async () => {
      try {
        const p = await invoke("scan_progress");
        renderScanProgress(p);
        if (!p.running) {
          stopScanPolling();
          btn.disabled = false;
          stopBtn.disabled = true;
          if (p.error) {
            $("#scanStatus").textContent = "扫描失败：" + p.error;
            setStatus("扫描失败");
          } else if (p.done) {
            const s = p.done;
            if (s.cancelled) {
              $("#scanStatus").textContent =
                `扫描已停止：已处理 ${s.found} 个文件（新增 ${s.new}，更新 ${s.updated}，跳过 ${s.skipped}，失败 ${s.failed}，自动删除重复 ${s.deleted_duplicates}）`;
              setStatus("扫描已停止（已处理文件已入库）");
            } else {
              $("#scanStatus").textContent =
                `扫描完成：发现 ${s.found}，新增 ${s.new}，更新 ${s.updated}，跳过 ${s.skipped}，` +
                `失败 ${s.failed}，自动删除重复 ${s.deleted_duplicates}，missing ${s.missing}，去重候选 ${s.duplicate_candidates}`;
              setStatus("扫描完成（数据库已更新）");
            }
          }
        }
      } catch (e) {
        stopScanPolling();
        btn.disabled = false;
        stopBtn.disabled = true;
        $("#scanStatus").textContent = "查询扫描进度失败：" + e;
        setStatus("扫描进度查询失败");
      }
    }, 300);
  } catch (e) {
    btn.disabled = false;
    stopBtn.disabled = true;
    $("#scanStatus").textContent = "启动扫描失败：" + e;
    setStatus("启动扫描失败");
  }
});

$("#stopScanBtn").addEventListener("click", async () => {
  try {
    await invoke("cancel_scan");
    setStatus("正在停止扫描……（处理完当前文件后结束）");
  } catch (e) {
    setStatus("停止扫描请求失败：" + e);
  }
});

// ---------- 文件库页 ----------

async function loadInstruments() {
  try {
    const list = await run("加载乐器列表", () => invoke("list_instruments"));
    state.instruments = list.map((x) => ({ ...x, checked: false }));
    renderInstrumentList();
    setStatus("乐器列表已加载（共 " + list.length + " 种）");
  } catch (e) {
    /* 状态栏已提示 */
  }
}

function renderInstrumentList() {
  const kw = state.instFilter.toLowerCase();
  const filtered = state.instruments.filter(
    (x) => !kw || x.name.toLowerCase().includes(kw),
  );
  $("#instList").innerHTML = filtered
    .map(
      (x) =>
        `<label><input type="checkbox" data-id="${x.id}" ${x.checked ? "checked" : ""}/> ${escapeHtml(x.name)}</label>`,
    )
    .join("");
  $("#instList")
    .querySelectorAll("input[type=checkbox]")
    .forEach((cb) => {
      cb.addEventListener("change", () => {
        const inst = state.instruments.find(
          (x) => x.id === Number(cb.dataset.id),
        );
        if (inst) inst.checked = cb.checked;
      });
    });
}

$("#instFilterInput").addEventListener("input", (e) => {
  state.instFilter = e.target.value;
  renderInstrumentList();
});

function selectedInstrumentIds() {
  return state.instruments.filter((x) => x.checked).map((x) => x.id);
}

function parseRange(v) {
  const s = v.trim();
  if (!s) return null;
  const parts = s.split("-");
  let lo = null;
  let hi = null;
  if (parts.length >= 1 && parts[0].trim() !== "") lo = Number(parts[0].trim());
  if (parts.length >= 2 && parts[1].trim() !== "") hi = Number(parts[1].trim());
  if (
    parts.length > 2 ||
    (lo !== null && isNaN(lo)) ||
    (hi !== null && isNaN(hi))
  ) {
    throw new Error("区间格式应为 min-max，如 100-5000");
  }
  if (lo !== null && hi !== null && lo > hi)
    throw new Error("下限不能大于上限");
  return { lo, hi };
}

function rangeArgs(r) {
  return r
    ? { min: r.lo ?? 0, max: r.hi ?? Number.MAX_SAFE_INTEGER }
    : { min: null, max: null };
}

async function runQuery() {
  const selectedIds = selectedInstrumentIds();
  const matchMode = document.querySelector(
    'input[name="matchMode"]:checked',
  ).value;
  let totalRange = null;
  let noteRange = null;
  try {
    totalRange = parseRange($("#totalRangeInput").value);
    noteRange = parseRange($("#noteRangeInput").value);
  } catch (e) {
    setStatus("输入有误：" + e.message);
    return;
  }
  const tr = rangeArgs(totalRange);
  const nr = rangeArgs(noteRange);
  const name = $("#nameInput").value.trim() || null;
  const dir = $("#dirInput").value.trim() || null;

  try {
    const rows = await run("查询中", () =>
      invoke("query", {
        selectedIds,
        matchMode,
        noteMin: selectedIds.length ? nr.min : null,
        noteMax: selectedIds.length ? nr.max : null,
        totalMin: tr.min,
        totalMax: tr.max,
        name,
        dir,
        page: state.page,
        pageSize: state.pageSize,
      }),
    );
    state.results = rows;
    renderResults();
    if (selectedIds.length === 0) {
      setStatus(`查询到 ${rows.length} 条`);
    } else if (matchMode === "superset") {
      setStatus(`查询到 ${rows.length} 条（包含所选全部乐器，允许含其他乐器）`);
    } else {
      setStatus(
        `查询到 ${rows.length} 条（乐器集合恰好等于所选 ${selectedIds.length} 种）`,
      );
    }
  } catch (e) {
    /* 状态栏已提示 */
  }
}

$("#queryBtn").addEventListener("click", () => {
  state.page = 0;
  runQuery();
});

function renderResults() {
  $("#resultTitle").textContent = `查询结果（第 ${state.page + 1} 页）`;
  $("#pageInfo").textContent =
    `第 ${state.page + 1} 页 · 每页 ${state.pageSize} 条`;
  if (state.results.length === 0) {
    $("#resultTableWrap").innerHTML = '<div class="status">无匹配结果</div>';
    return;
  }
  const rows = state.results
    .map((r, i) => {
      const insts = r.instruments
        .map((x) => `${escapeHtml(x.name)}:${x.note_count}`)
        .join(", ");
      return `<tr data-idx="${i}" class="${i === 0 ? "selected" : ""}">
        <td>${escapeHtml(r.file_name)}</td>
        <td title="${escapeHtml(r.path)}">${escapeHtml(r.path)}</td>
        <td>${r.note_total}</td>
        <td>${insts}</td>
        <td><button class="small" data-open="${i}">打开</button></td>
      </tr>`;
    })
    .join("");
  $("#resultTableWrap").innerHTML = `<table>
    <thead><tr><th>文件名</th><th>路径</th><th>总音符</th><th>乐器</th><th></th></tr></thead>
    <tbody>${rows}</tbody></table>`;
  $("#resultTableWrap")
    .querySelectorAll("button[data-open]")
    .forEach((b) => {
      b.addEventListener("click", () => {
        const r = state.results[Number(b.dataset.open)];
        if (r)
          invoke("open_file", { path: r.path }).catch((e) =>
            setStatus("打开失败：" + e),
          );
      });
    });
}

$("#prevPageBtn").addEventListener("click", () => {
  if (state.page > 0) {
    state.page -= 1;
    runQuery();
  }
});

$("#nextPageBtn").addEventListener("click", () => {
  state.page += 1;
  runQuery();
});

// ---------- 去重中心 ----------

async function refreshGroups() {
  try {
    const groups = await run("加载去重候选", () => invoke("pending_groups"));
    state.groups = groups;
    state.expandedGroup = null;
    state.memberChecked = [];
    renderGroups();
    setStatus("待处理候选组：" + groups.length);
  } catch (e) {
    /* 状态栏已提示 */
  }
}

$("#refreshDedupBtn").addEventListener("click", refreshGroups);

// 流式去重检测：后台逐组产出 → 前端轮询增量 append；检测中可点同一按钮停止；可边收边删
let detectTimer = null;

function stopDetectPolling() {
  if (detectTimer !== null) {
    clearInterval(detectTimer);
    detectTimer = null;
  }
}

$("#detectBtn").addEventListener("click", async () => {
  const btn = $("#detectBtn");
  // 检测运行中：点击 = 停止
  if (btn.dataset.running === "1") {
    try {
      await invoke("cancel_detect");
      setStatus("正在停止检测……（处理完当前指纹后结束）");
    } catch (e) {
      setStatus("停止检测失败：" + e);
    }
    return;
  }

  btn.dataset.running = "1";
  btn.textContent = "停止检测";
  btn.classList.add("danger");
  try {
    await invoke("detect_duplicates");
    setStatus("正在检测（一次性全量）……结果实时追加，可边收边删");
    stopDetectPolling();
    detectTimer = setInterval(async () => {
      try {
        const p = await invoke("detect_progress");
        if (p.new_groups && p.new_groups.length) {
          appendGroups(p.new_groups);
        }
        if (p.running) {
          setStatus(
            `检测中：已生成 ${p.processed_groups} 个候选组 / ${p.processed_files} 个候选文件`,
          );
        } else {
          stopDetectPolling();
          btn.dataset.running = "0";
          btn.textContent = "重新检测重复";
          btn.classList.remove("danger");
          if (p.error) {
            setStatus("检测失败：" + p.error);
          } else if (p.done) {
            const d = p.done;
            const cancelMsg = d.cancelled
              ? "；已手动停止（未检测完部分可稍后重新检测）"
              : "";
            setStatus(
              `检测完成：${d.groups} 个候选组 / ${d.candidates} 个候选文件${cancelMsg}`,
            );
          }
        }
      } catch (e) {
        stopDetectPolling();
        btn.dataset.running = "0";
        btn.textContent = "重新检测重复";
        btn.classList.remove("danger");
        setStatus("查询检测进度失败：" + e);
      }
    }, 300);
  } catch (e) {
    btn.dataset.running = "0";
    btn.textContent = "重新检测重复";
    btn.classList.remove("danger");
    setStatus("启动检测失败：" + e);
  }
});

// 一键全部去重：后台分批处理（可停止、实时进度）
let resolveTimer = null;

function stopResolvePolling() {
  if (resolveTimer !== null) {
    clearInterval(resolveTimer);
    resolveTimer = null;
  }
}

function renderResolveProgress(p) {
  const el = $("#resolveProgress");
  if (p.running) {
    const frame = SPIN[Math.floor(Date.now() / 200) % SPIN.length];
    el.innerHTML =
      `<span class="spin">${frame}</span> 全部去重中：已处理 ${p.processed_groups} / ${p.total_groups} 组 · ` +
      `已删除 ${p.deleted_files} 个文件（第 ${p.current_group} 组）`;
  } else {
    el.textContent = "";
  }
}

// 一键全部去重：确认弹窗
$("#resolveAllBtn").addEventListener("click", () => {
  if (state.groups.length === 0) {
    setStatus("没有待处理的候选组");
    return;
  }
  const totalDeletes = state.groups.reduce(
    (sum, g) => sum + (g.member_count - 1),
    0,
  );
  $("#confirmTitle").textContent =
    `确认全部去重（${state.groups.length} 个候选组）？`;
  $("#confirmBody").textContent =
    `按默认规则分批清理：每组保留最早入库的文件，删除其余。\n` +
    `当前共 ${state.groups.length} 个候选组、约 ${totalDeletes} 个待删文件；` +
    `删除为永久操作，不可恢复；后台执行，可随时停止。确认开始本批？`;
  state.pendingDelete = { all: true };
  $("#confirmModal").classList.remove("hidden");
});

// 停止全部去重
$("#stopResolveBtn").addEventListener("click", async () => {
  try {
    await invoke("cancel_resolve");
    setStatus("正在停止……（处理完当前组后结束）");
  } catch (e) {
    setStatus("停止请求失败：" + e);
  }
});

// 清空候选：只清状态、不删文件（确认弹窗）
$("#clearPendingBtn").addEventListener("click", () => {
  if (state.groups.length === 0) {
    setStatus("没有待处理的候选组");
    return;
  }
  const totalMembers = state.groups.reduce((s, g) => s + g.member_count, 0);
  $("#confirmTitle").textContent =
    `确认清空候选（${state.groups.length} 组）？`;
  $("#confirmBody").textContent =
    `将把全部 ${state.groups.length} 个待处理候选组标记为已忽略，` +
    `${totalMembers} 个文件恢复为「已扫描」状态。\n` +
    `不会删除任何文件。确认？`;
  state.pendingDelete = { clearAll: true };
  $("#confirmModal").classList.remove("hidden");
});

function groupCardHtml(g, gi) {
  const tag =
    g.dup_type === "byte_identical"
      ? '<span class="tag byte">字节相同</span>'
      : '<span class="tag struct">结构相同</span>';
  const membersHtml = state.expandedGroup === gi ? renderMembers(g, gi) : "";
  return `<div class="group-card">
    <div class="group-head" data-group="${gi}">
      <span>[${state.expandedGroup === gi ? "v" : " "}]</span>
      <span>组 ${gi + 1}</span>
      ${tag}
      <span class="hint">${g.member_count} 个成员 · 指纹 ${g.fingerprint.slice(0, 12)}…</span>
    </div>
    ${membersHtml}
  </div>`;
}

function bindGroupHeadEvents() {
  $("#dedupList")
    .querySelectorAll(".group-head")
    .forEach((el) => {
      if (el.dataset.bound) return;
      el.dataset.bound = "1";
      el.addEventListener("click", () => {
        const gi = Number(el.dataset.group);
        if (state.expandedGroup === gi) {
          state.expandedGroup = null;
        } else {
          state.expandedGroup = gi;
          // 默认：最早入库（第一个成员）保留，其余勾选删除（D9）
          const g = state.groups[gi];
          state.memberChecked = g.members.map((_, i) => i !== 0);
        }
        renderGroups();
      });
    });
}

function renderGroups() {
  if (state.groups.length === 0) {
    $("#dedupList").innerHTML =
      '<div class="status">没有待处理的去重候选组</div>';
    return;
  }
  $("#dedupList").innerHTML = state.groups
    .map((g, gi) => groupCardHtml(g, gi))
    .join("");
  bindGroupHeadEvents();
}

// 流式检测：把后端新产出的组 append 到列表（边收边展示）
function appendGroups(newGroups) {
  if (!newGroups || newGroups.length === 0) return;
  let html = "";
  for (const g of newGroups) {
    const gi = state.groups.length;
    state.groups.push(g);
    html += groupCardHtml(g, gi);
  }
  const listEl = $("#dedupList");
  // 若列表当前是空占位文本，先清掉再追加
  if (!listEl.querySelector(".group-card")) {
    listEl.innerHTML = "";
  }
  listEl.insertAdjacentHTML("beforeend", html);
  bindGroupHeadEvents();
}

function renderMembers(g, gi) {
  const rows = g.members
    .map((m, i) => {
      const locked = i === 0;
      const checked = state.memberChecked[i] ? "checked" : "";
      const tag = locked ? '<span class="tag">保留</span>' : "";
      return `<label class="${locked ? "locked" : ""}">
        <input type="checkbox" data-g="${gi}" data-m="${i}" ${checked} ${locked ? "disabled" : ""}/>
        #${m.id} ${escapeHtml(m.path)}（${m.note_total} 音符，${m.size_bytes} 字节）${tag}
      </label>`;
    })
    .join("");
  const count = state.memberChecked.filter(Boolean).length;
  return `<div class="member-list">
    ${rows}
    <div class="row">
      <button class="small" data-all="${gi}">全选（除保留）</button>
      <button class="small danger" data-del="${gi}">确认删除（${count} 个）</button>
    </div>
  </div>`;
}

$("#dedupList").addEventListener("change", (e) => {
  const cb = e.target;
  if (cb.matches("input[type=checkbox][data-g]")) {
    const gi = Number(cb.dataset.g);
    const mi = Number(cb.dataset.m);
    state.memberChecked[mi] = cb.checked;
  }
});

$("#dedupList").addEventListener("click", (e) => {
  const allBtn = e.target.closest("button[data-all]");
  const delBtn = e.target.closest("button[data-del]");
  if (allBtn) {
    const gi = Number(allBtn.dataset.all);
    const g = state.groups[gi];
    state.memberChecked = g.members.map((_, i) => i !== 0);
    renderGroups();
    return;
  }
  if (delBtn) {
    const gi = Number(delBtn.dataset.del);
    const g = state.groups[gi];
    const deleteIds = g.members
      .map((m, i) => (state.memberChecked[i] ? m.id : null))
      .filter((x) => x !== null);
    if (deleteIds.length === 0) {
      setStatus("未勾选任何要删除的文件");
      return;
    }
    state.pendingDelete = {
      groupId: g.id,
      keepId: g.members[0].id,
      deleteIds,
      paths: g.members
        .filter((_, i) => state.memberChecked[i])
        .map((m) => m.path),
    };
    $("#confirmTitle").textContent =
      `确认永久删除 ${deleteIds.length} 个文件？`;
    $("#confirmBody").textContent = state.pendingDelete.paths.join("\n");
    $("#confirmModal").classList.remove("hidden");
  }
});

$("#confirmYes").addEventListener("click", async () => {
  const p = state.pendingDelete;
  $("#confirmModal").classList.add("hidden");
  state.pendingDelete = null;
  if (!p) return;

  // 清空候选（只清状态、不删文件）
  if (p.clearAll) {
    try {
      const out = await run("清空候选中", () => invoke("clear_pending_groups"));
      setStatus(
        `已清空候选：${out.dismissed_groups} 组标记已忽略，${out.restored_files} 个文件恢复为已扫描（未删除任何文件）`,
      );
      await refreshGroups();
    } catch (e) {
      setStatus("清空候选失败：" + e);
    }
    return;
  }

  // 一键全部去重（后台分批 + 轮询进度 + 可停止）
  if (p.all) {
    const btn = $("#resolveAllBtn");
    const stopBtn = $("#stopResolveBtn");
    btn.disabled = true;
    stopBtn.disabled = false;
    try {
      await invoke("resolve_all_groups");
      setStatus("全部去重中……（可随时停止）");
      stopResolvePolling();
      resolveTimer = setInterval(async () => {
        try {
          const r = await invoke("resolve_progress");
          renderResolveProgress(r);
          if (!r.running) {
            stopResolvePolling();
            btn.disabled = false;
            stopBtn.disabled = true;
            if (r.error) {
              setStatus("全部去重失败：" + r.error);
            } else if (r.done) {
              const d = r.done;
              const errMsg =
                d.errors && d.errors.length
                  ? `；失败 ${d.errors.length} 组（${d.errors.join("；")}）`
                  : "";
              const cancelMsg = d.cancelled ? "；已手动停止" : "";
              const remainMsg =
                d.remaining_groups > 0
                  ? `；还有 ${d.remaining_groups} 组待处理，可再次点「全部去重」继续下一批`
                  : "；全部候选组已处理完";
              setStatus(
                `本批处理 ${d.resolved_groups} 组，删除 ${d.deleted_files} 个文件${errMsg}${cancelMsg}${remainMsg}`,
              );
              await refreshGroups();
            }
          }
        } catch (e) {
          stopResolvePolling();
          btn.disabled = false;
          stopBtn.disabled = true;
          setStatus("查询全部去重进度失败：" + e);
        }
      }, 300);
    } catch (e) {
      btn.disabled = false;
      stopBtn.disabled = true;
      setStatus("启动全部去重失败：" + e);
    }
    return;
  }

  // 单组删除
  try {
    const out = await run("删除中", () =>
      invoke("resolve_group", {
        groupId: p.groupId,
        keepId: p.keepId,
        deleteIds: p.deleteIds,
      }),
    );
    setStatus(`已硬删 ${out.deleted} 个文件，候选组已解决`);
    await refreshGroups();
  } catch (e) {
    setStatus("删除失败：" + e);
  }
});

$("#confirmNo").addEventListener("click", () => {
  $("#confirmModal").classList.add("hidden");
  state.pendingDelete = null;
});

// ---------- 设置统计 ----------

async function refreshStats() {
  try {
    const s = await run("加载统计", () => invoke("stats"));
    const counts = s.counts
      .map((c) => `<div>${escapeHtml(c.status)}: ${c.count}</div>`)
      .join("");
    const top = s.instrument_top
      .map((x) => `<div>${escapeHtml(x.name)}: ${x.count} 个文件</div>`)
      .join("");
    $("#statsView").innerHTML = `
      <div class="status">数据库: ${escapeHtml(s.db_path)}</div>
      <h3>文件状态统计</h3>
      <div>${counts || "（空库）"}</div>
      <h3>乐器分布 Top 10</h3>
      <div>${top || "（空库）"}</div>`;
    setStatus("统计已刷新");
  } catch (e) {
    /* 状态栏已提示 */
  }
}

$("#refreshStatsBtn").addEventListener("click", refreshStats);

// ---------- 页面导航 ----------

document.querySelectorAll("nav button").forEach((b) => {
  b.addEventListener("click", () => switchPage(b.dataset.page));
});

function escapeHtml(s) {
  return String(s).replace(
    /[&<>"']/g,
    (c) =>
      ({
        "&": "&amp;",
        "<": "&lt;",
        ">": "&gt;",
        '"': "&quot;",
        "'": "&#39;",
      })[c],
  );
}

// 初始加载
setStatus(
  "就绪：F 键无冲突，直接点击操作；数据库默认 ~/.midi-manager/library.sqlite",
);
