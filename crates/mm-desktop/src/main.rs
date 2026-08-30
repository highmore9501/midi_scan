//! MIDI Manager 桌面应用（Tauri 2）：Rust 命令层，前端为 Web（ui/ 目录）。

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

use midi_scan::InstrumentId;
use mm_core::query::QueryParams;
use mm_core::service::Service;
use tauri::State;
use tauri_plugin_dialog::DialogExt;

struct AppState {
    db_path: PathBuf,
    scan_job: Arc<Mutex<ScanJob>>,
    resolve_job: Arc<Mutex<ResolveJob>>,
    detect_job: Arc<Mutex<DetectJob>>,
}

/// 后台扫描任务状态（供前端轮询进度）
#[derive(Default)]
struct ScanJob {
    running: bool,
    cancel: Arc<AtomicBool>,
    current_file: String,
    found: u64,
    new: u64,
    updated: u64,
    skipped: u64,
    failed: u64,
    deleted_duplicates: u64,
    done: Option<serde_json::Value>,
    error: Option<String>,
}

/// 后台「全部去重」任务状态（供前端轮询进度，可停止）
#[derive(Default)]
struct ResolveJob {
    running: bool,
    cancel: Arc<AtomicBool>,
    total_groups: usize,
    processed_groups: u64,
    deleted_files: u64,
    current_group: usize,
    done: Option<serde_json::Value>,
    error: Option<String>,
}

/// 后台流式去重检测任务：逐组产出后立即入队，前端轮询 append 展示，可边收边删
#[derive(Default)]
struct DetectJob {
    running: bool,
    cancel: Arc<AtomicBool>,
    processed_groups: u64,
    processed_files: u64,
    new_groups: Vec<serde_json::Value>,
    done: Option<serde_json::Value>,
    error: Option<String>,
}

fn default_db_path() -> PathBuf {
    let base = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base).join(".midi-manager").join("library.sqlite")
}

// ---------- 命令 ----------

/// 启动后台扫描线程（非阻塞）；进度通过 `scan_progress` 轮询，可随时 `cancel_scan` 停止
#[tauri::command]
fn scan(
    state: State<AppState>,
    dirs: Vec<String>,
    auto_dedup: Option<String>,
) -> Result<serde_json::Value, String> {
    let auto_dedup = match auto_dedup.as_deref() {
        Some("structure") => mm_core::scan::AutoDedupMode::Structure,
        Some("off") => mm_core::scan::AutoDedupMode::Off,
        _ => mm_core::scan::AutoDedupMode::Byte, // GUI 默认：字节相同直接删除
    };
    let cancel = {
        let mut job = state.scan_job.lock().unwrap();
        if job.running {
            return Err("扫描已在运行中".to_string());
        }
        *job = ScanJob::default();
        job.running = true;
        job.cancel.clone()
    };
    let job = state.scan_job.clone();
    let db_path = state.db_path.clone();
    std::thread::spawn(move || {
        let result = (|| -> Result<serde_json::Value, String> {
            let mut svc = Service::open(&db_path).map_err(|e| e.to_string())?;
            let roots: Vec<PathBuf> = dirs.into_iter().map(PathBuf::from).collect();
            let mut progress = |p: &mm_core::scan::ScanProgress| {
                let mut j = job.lock().unwrap();
                j.current_file = p.current_file.clone();
                j.found = p.found;
                j.new = p.new;
                j.updated = p.updated;
                j.skipped = p.skipped;
                j.failed = p.failed;
                j.deleted_duplicates = p.deleted_duplicates;
            };
            let s = svc
                .scan(&roots, true, auto_dedup, Some(&mut progress), Some(&cancel))
                .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "found": s.found,
                "new": s.new,
                "updated": s.updated,
                "skipped": s.skipped,
                "failed": s.failed,
                "missing": s.missing,
                "deleted_duplicates": s.deleted_duplicates,
                "duplicate_candidates": s.duplicate_candidates,
                "cancelled": s.cancelled,
            }))
        })();
        let mut j = job.lock().unwrap();
        match result {
            Ok(v) => j.done = Some(v),
            Err(e) => j.error = Some(e),
        }
        j.running = false;
    });
    Ok(serde_json::json!({ "started": true }))
}

/// 请求停止当前扫描（扫描线程在下一个文件处检查并提前结束）
#[tauri::command]
fn cancel_scan(state: State<AppState>) -> Result<(), String> {
    let job = state.scan_job.lock().unwrap();
    job.cancel.store(true, Ordering::Relaxed);
    Ok(())
}

/// 查询后台扫描进度
#[tauri::command]
fn scan_progress(state: State<AppState>) -> Result<serde_json::Value, String> {
    let j = state.scan_job.lock().unwrap();
    Ok(serde_json::json!({
        "running": j.running,
        "cancelled": j.cancel.load(Ordering::Relaxed),
        "current_file": j.current_file,
        "found": j.found,
        "new": j.new,
        "updated": j.updated,
        "skipped": j.skipped,
        "failed": j.failed,
        "deleted_duplicates": j.deleted_duplicates,
        "done": j.done,
        "error": j.error,
    }))
}

/// 弹出系统原生目录选择对话框
#[tauri::command]
fn pick_folder(window: tauri::Window) -> Result<Option<String>, String> {
    let picked = window.dialog().file().blocking_pick_folder();
    Ok(picked.map(|p| p.to_string()))
}

/// 库里出现过的乐器列表（查询页多选用）
#[tauri::command]
fn list_instruments(state: State<AppState>) -> Result<serde_json::Value, String> {
    let svc = Service::open(&state.db_path).map_err(|e| e.to_string())?;
    let list = svc.db.list_instruments().map_err(|e| e.to_string())?;
    let arr: Vec<_> = list
        .into_iter()
        .map(|(id, bmsb, blsb, prog, perc, name)| {
            serde_json::json!({
                "id": id,
                "bank_msb": bmsb,
                "bank_lsb": blsb,
                "program": prog,
                "is_percussion": perc != 0,
                "name": name,
            })
        })
        .collect();
    Ok(serde_json::json!(arr))
}

/// Exact 组合查询（D6）
#[allow(clippy::too_many_arguments)]
#[tauri::command]
fn query(
    state: State<AppState>,
    selected_ids: Vec<i64>,
    match_mode: Option<String>,
    note_min: Option<u64>,
    note_max: Option<u64>,
    total_min: Option<u64>,
    total_max: Option<u64>,
    name: Option<String>,
    dir: Option<String>,
    page: u64,
    page_size: u64,
) -> Result<serde_json::Value, String> {
    let svc = Service::open(&state.db_path).map_err(|e| e.to_string())?;
    let list = svc.db.list_instruments().map_err(|e| e.to_string())?;
    let mut selected = Vec::new();
    for (id, bmsb, blsb, prog, perc, _) in &list {
        if selected_ids.contains(id) {
            selected.push(InstrumentId {
                bank_msb: *bmsb as u8,
                bank_lsb: *blsb as u8,
                program: *prog as u8,
                is_percussion: *perc != 0,
            });
        }
    }

    let match_mode = match match_mode.as_deref() {
        Some("superset") => mm_core::query::MatchMode::Superset,
        _ => mm_core::query::MatchMode::Exact,
    };

    let mut per_instrument_note_range = HashMap::new();
    if !selected.is_empty() && (note_min.is_some() || note_max.is_some()) {
        let range = (note_min.unwrap_or(0), note_max.unwrap_or(u64::MAX));
        for inst in &selected {
            per_instrument_note_range.insert(*inst, range);
        }
    }
    let total_range = match (total_min, total_max) {
        (None, None) => None,
        (lo, hi) => Some((lo.unwrap_or(0), hi.unwrap_or(u64::MAX))),
    };

    let params = QueryParams {
        selected,
        match_mode,
        per_instrument_note_range,
        total_note_range: total_range,
        name_keyword: name,
        dir_filter: dir,
        limit: page_size,
        offset: page * page_size,
    };
    let rows = svc.query(&params).map_err(|e| e.to_string())?;
    let arr: Vec<_> = rows
        .into_iter()
        .map(|r| {
            serde_json::json!({
                "id": r.id,
                "path": r.path,
                "file_name": r.file_name,
                "note_total": r.note_total,
                "instruments": r.instruments.iter()
                    .map(|(n, c)| serde_json::json!({"name": n, "note_count": c}))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(serde_json::json!(arr))
}

/// 流式去重检测：后台逐指纹处理，每建一个候选组立即入队；
/// 前端通过 `detect_progress` 轮询增量获取（new_groups 取走即清空），可随时 `cancel_detect` 停止，
/// 也可边检测边执行删除操作（WAL + 最新快照，并发安全）。
#[tauri::command]
fn detect_duplicates(state: State<AppState>) -> Result<serde_json::Value, String> {
    let cancel = {
        let mut job = state.detect_job.lock().unwrap();
        if job.running {
            return Err("检测已在运行中".to_string());
        }
        *job = DetectJob::default();
        job.running = true;
        job.cancel.clone()
    };
    let job = state.detect_job.clone();
    let db_path = state.db_path.clone();
    std::thread::spawn(move || {
        let result = (|| -> Result<serde_json::Value, String> {
            let mut svc = Service::open(&db_path).map_err(|e| e.to_string())?;
            let outcome = mm_core::duplicate::detect_streaming(
                &mut svc.db,
                mm_core::duplicate::DETECT_BATCH_LIMIT,
                &cancel,
                |group| {
                    let mut j = job.lock().unwrap();
                    j.processed_groups += 1;
                    j.processed_files += group.member_count as u64;
                    j.new_groups.push(dup_group_json(group));
                    Ok(())
                },
            )
            .map_err(|e| e.to_string())?;
            Ok(serde_json::json!({
                "groups": outcome.processed_groups,
                "candidates": outcome.processed_files,
                "remaining": outcome.remaining_groups,
                "cancelled": cancel.load(Ordering::Relaxed),
            }))
        })();
        let mut j = job.lock().unwrap();
        match result {
            Ok(v) => j.done = Some(v),
            Err(e) => j.error = Some(e),
        }
        j.running = false;
    });
    Ok(serde_json::json!({ "started": true }))
}

/// 查询流式检测进度；`new_groups` 为本次轮询新增的候选组（取走即清空），前端 append 展示
#[tauri::command]
fn detect_progress(state: State<AppState>) -> Result<serde_json::Value, String> {
    let mut j = state.detect_job.lock().unwrap();
    let new_groups = std::mem::take(&mut j.new_groups);
    Ok(serde_json::json!({
        "running": j.running,
        "cancelled": j.cancel.load(Ordering::Relaxed),
        "processed_groups": j.processed_groups,
        "processed_files": j.processed_files,
        "new_groups": new_groups,
        "done": j.done,
        "error": j.error,
    }))
}

/// 停止流式去重检测
#[tauri::command]
fn cancel_detect(state: State<AppState>) -> Result<(), String> {
    let job = state.detect_job.lock().unwrap();
    job.cancel.store(true, Ordering::Relaxed);
    Ok(())
}

fn dup_group_json(g: &mm_core::duplicate::DupGroup) -> serde_json::Value {
    serde_json::json!({
        "id": g.id,
        "fingerprint": g.fingerprint,
        "dup_type": g.dup_type,
        "member_count": g.member_count,
        "members": g.members.iter().map(|m| serde_json::json!({
            "id": m.id,
            "path": m.path,
            "file_name": m.file_name,
            "note_total": m.note_total,
            "size_bytes": m.size_bytes,
            "status": m.status,
            "first_scanned_at": m.first_scanned_at,
        })).collect::<Vec<_>>(),
    })
}

/// 待处理的去重候选组（D7/D9）
#[tauri::command]
fn pending_groups(state: State<AppState>) -> Result<serde_json::Value, String> {
    let svc = Service::open(&state.db_path).map_err(|e| e.to_string())?;
    let groups = svc.pending_groups().map_err(|e| e.to_string())?;
    let arr: Vec<_> = groups
        .into_iter()
        .map(|g| {
            serde_json::json!({
                "id": g.id,
                "fingerprint": g.fingerprint,
                "dup_type": g.dup_type,
                "member_count": g.member_count,
                "members": g.members.iter().map(|m| serde_json::json!({
                    "id": m.id,
                    "path": m.path,
                    "file_name": m.file_name,
                    "note_total": m.note_total,
                    "size_bytes": m.size_bytes,
                    "status": m.status,
                    "first_scanned_at": m.first_scanned_at,
                })).collect::<Vec<_>>(),
            })
        })
        .collect();
    Ok(serde_json::json!(arr))
}

/// 确认后硬删（D10）
#[tauri::command]
fn resolve_group(
    state: State<AppState>,
    group_id: i64,
    keep_id: i64,
    delete_ids: Vec<i64>,
) -> Result<serde_json::Value, String> {
    let mut svc = Service::open(&state.db_path).map_err(|e| e.to_string())?;
    let out = svc
        .resolve_group(group_id, keep_id, &delete_ids)
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({ "deleted": out.deleted }))
}

/// 一键全部去重：后台线程按默认规则处理所有待处理候选组——
/// 每组保留最早入库（id 最小）的文件，其余硬删（D9/D10）。
/// 进度通过 `resolve_progress` 轮询，可随时 `cancel_resolve` 停止。
#[tauri::command]
fn resolve_all_groups(state: State<AppState>) -> Result<serde_json::Value, String> {
    let cancel = {
        let mut job = state.resolve_job.lock().unwrap();
        if job.running {
            return Err("全部去重已在运行中".to_string());
        }
        *job = ResolveJob::default();
        job.running = true;
        job.cancel.clone()
    };
    let job = state.resolve_job.clone();
    let db_path = state.db_path.clone();
    std::thread::spawn(move || {
        let result = (|| -> Result<serde_json::Value, String> {
            let mut svc = Service::open(&db_path).map_err(|e| e.to_string())?;
            let groups = svc.pending_groups().map_err(|e| e.to_string())?;
            {
                let mut j = job.lock().unwrap();
                j.total_groups = groups.len();
            }
            let mut processed = 0u64;
            let mut deleted = 0u64;
            let mut errors: Vec<String> = Vec::new();
            for (idx, g) in groups.iter().enumerate() {
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
                let keep_id = g.members[0].id; // 最早入库
                let deletes: Vec<i64> = g
                    .members
                    .iter()
                    .map(|m| m.id)
                    .filter(|id| *id != keep_id)
                    .collect();
                match svc.resolve_group(g.id, keep_id, &deletes) {
                    Ok(out) => {
                        processed += 1;
                        deleted += out.deleted;
                    }
                    Err(e) => errors.push(format!("组 {}: {e}", g.id)),
                }
                let mut j = job.lock().unwrap();
                j.processed_groups = processed;
                j.deleted_files = deleted;
                j.current_group = idx + 1;
            }
            Ok(serde_json::json!({
                "resolved_groups": processed,
                "deleted_files": deleted,
                "errors": errors,
                "cancelled": cancel.load(Ordering::Relaxed),
            }))
        })();
        let mut j = job.lock().unwrap();
        match result {
            Ok(v) => j.done = Some(v),
            Err(e) => j.error = Some(e),
        }
        j.running = false;
    });
    Ok(serde_json::json!({ "started": true }))
}

/// 查询后台「全部去重」进度
#[tauri::command]
fn resolve_progress(state: State<AppState>) -> Result<serde_json::Value, String> {
    let j = state.resolve_job.lock().unwrap();
    Ok(serde_json::json!({
        "running": j.running,
        "cancelled": j.cancel.load(Ordering::Relaxed),
        "total_groups": j.total_groups,
        "processed_groups": j.processed_groups,
        "deleted_files": j.deleted_files,
        "current_group": j.current_group,
        "done": j.done,
        "error": j.error,
    }))
}

/// 停止后台「全部去重」
#[tauri::command]
fn cancel_resolve(state: State<AppState>) -> Result<(), String> {
    let job = state.resolve_job.lock().unwrap();
    job.cancel.store(true, Ordering::Relaxed);
    Ok(())
}

/// 清空全部待处理候选（不删除任何文件）：候选组标记 dismissed，文件状态恢复 scanned
#[tauri::command]
fn clear_pending_groups(state: State<AppState>) -> Result<serde_json::Value, String> {
    let mut svc = Service::open(&state.db_path).map_err(|e| e.to_string())?;
    let dismissed = svc
        .db
        .dismiss_all_pending_groups()
        .map_err(|e| e.to_string())?;
    let restored = svc
        .db
        .reset_stale_candidates()
        .map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "dismissed_groups": dismissed,
        "restored_files": restored,
    }))
}

/// 调用系统默认程序打开文件
#[tauri::command]
fn open_file(path: String) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("cmd")
            .args(["/c", "start", "", &path])
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        let cmd = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
        std::process::Command::new(cmd)
            .arg(&path)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// 统计信息（设置页）
#[tauri::command]
fn stats(state: State<AppState>) -> Result<serde_json::Value, String> {
    let svc = Service::open(&state.db_path).map_err(|e| e.to_string())?;
    let counts = svc.db.counts_by_status().map_err(|e| e.to_string())?;
    let top = svc.db.instrument_top(10).map_err(|e| e.to_string())?;
    Ok(serde_json::json!({
        "db_path": state.db_path.to_string_lossy(),
        "counts": counts.iter()
            .map(|(s, c)| serde_json::json!({"status": s, "count": c}))
            .collect::<Vec<_>>(),
        "instrument_top": top.iter()
            .map(|(n, c)| serde_json::json!({"name": n, "count": c}))
            .collect::<Vec<_>>(),
    }))
}

// ---------- 入口 ----------

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState {
            db_path: default_db_path(),
            scan_job: Arc::new(Mutex::new(ScanJob::default())),
            resolve_job: Arc::new(Mutex::new(ResolveJob::default())),
            detect_job: Arc::new(Mutex::new(DetectJob::default())),
        })
        .invoke_handler(tauri::generate_handler![
            scan,
            cancel_scan,
            scan_progress,
            pick_folder,
            list_instruments,
            query,
            pending_groups,
            detect_duplicates,
            detect_progress,
            cancel_detect,
            resolve_group,
            resolve_all_groups,
            resolve_progress,
            cancel_resolve,
            clear_pending_groups,
            open_file,
            stats
        ])
        .run(tauri::generate_context!())
        .expect("运行 Tauri 应用失败");
}
