//! 扫描器：增量扫描（F1+F3）、查重三层防线（F4）、快照对账、扫描期自动去重。

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

use mm_db::{Repository, ScannedFileInput, ScannedInstrumentInput};
use tracing::{info, warn};
use walkdir::WalkDir;
use xxhash_rust::xxh3::xxh3_64;

use crate::duplicate;
use crate::CoreError;

/// 扫描期的自动去重策略（v0.11 起仅按字节完全相同判定重复）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AutoDedupMode {
    /// 关闭：重复文件照常入库并进入候选组，由用户处理
    #[default]
    Off,
    /// 字节完全相同（content_hash 一致）时直接删除新发现的重复文件，不入库
    Byte,
}

#[derive(Debug, Default, Clone)]
pub struct ScanSummary {
    pub found: u64,
    pub new: u64,
    pub updated: u64,
    pub skipped: u64,
    pub failed: u64,
    pub missing: u64,
    /// 扫描期间自动删除的重复文件数（不入库）
    pub deleted_duplicates: u64,
    pub duplicate_candidates: u64,
    /// 是否被用户主动停止（停止时已处理文件保留入库，不做对账/去重）
    pub cancelled: bool,
}

/// 扫描过程中的进度快照（供 UI 实时展示当前正在扫描的文件）
#[derive(Debug, Clone, Default)]
pub struct ScanProgress {
    pub current_file: String,
    pub found: u64,
    pub new: u64,
    pub updated: u64,
    pub skipped: u64,
    pub failed: u64,
    pub deleted_duplicates: u64,
}

/// 扫描一个或多个目录并入库；`run_dedup` 控制扫描后是否执行全库去重检测（D7）。
/// `auto_dedup` 控制扫描期自动去重（Byte：字节相同直接删；Structure：结构相同也删；Off：进候选组）。
/// `progress` 为可选的进度回调（在扫描线程内同步调用）；
/// `cancel` 为可选的取消标志（UI 线程置 true 后，扫描线程在下一个文件处提前结束）。
pub fn scan_roots(
    db: &mut Repository,
    roots: &[PathBuf],
    run_dedup: bool,
    auto_dedup: AutoDedupMode,
    mut progress: Option<&mut dyn FnMut(&ScanProgress)>,
    cancel: Option<&std::sync::atomic::AtomicBool>,
) -> Result<ScanSummary, CoreError> {
    let mut report = |p: &ScanProgress| {
        if let Some(cb) = progress.as_deref_mut() {
            cb(p);
        }
    };
    // 进度上报宏：在调用点构造快照（避免闭包长期借用 summary 与循环内更新冲突；
    // summary / path 作为参数传入，规避宏卫生问题）
    macro_rules! report_progress {
        ($summary:expr, $path:expr) => {
            report(&ScanProgress {
                current_file: $path.to_string(),
                found: $summary.found,
                new: $summary.new,
                updated: $summary.updated,
                skipped: $summary.skipped,
                failed: $summary.failed,
                deleted_duplicates: $summary.deleted_duplicates,
            })
        };
    }
    let is_cancelled = || cancel.is_some_and(|c| c.load(std::sync::atomic::Ordering::Relaxed));
    for root in roots {
        if !root.is_dir() {
            return Err(CoreError::DirNotFound(root.clone()));
        }
    }

    let mut summary = ScanSummary::default();
    let mut present: HashSet<String> = HashSet::new();

    'roots: for root in roots {
        let root_str = root.to_string_lossy().into_owned();
        let walker = WalkDir::new(root).into_iter().filter_entry(|e| {
            if e.depth() == 0 {
                return true;
            }
            if e.file_type().is_dir() {
                return !is_hidden_or_system(e.path());
            }
            true
        });

        for entry in walker.filter_map(|e| e.ok()) {
            // 取消检查：用户停止扫描时提前结束。
            // 已处理文件保留入库；跳过对账（避免误标 missing），但仍执行末尾的去重检测。
            if is_cancelled() {
                summary.cancelled = true;
                break 'roots;
            }
            if entry.file_type().is_dir() {
                continue;
            }
            let path = entry.path();
            if !is_midi_file(path) || is_hidden_or_system(path) {
                continue;
            }
            let path_str = path.to_string_lossy().into_owned();
            present.insert(path_str.clone());
            summary.found += 1;

            let meta = match std::fs::metadata(path) {
                Ok(m) => m,
                Err(e) => {
                    warn!(file_path = %path_str, error = %e, "读取元数据失败");
                    summary.failed += 1;
                    report_progress!(summary, &path_str);
                    continue;
                }
            };
            let size = meta.len() as i64;
            let mtime_ns = mtime_nanos(&meta);

            let existing = db.find_by_path(&path_str)?;
            // 查重 L1：同路径 + 元数据未变 → 跳过（不再解析）。
            // failed 也跳过（避免反复重试，可手动重扫）；
            // missing/deleted 即使元数据相同也要重新解析（文件重新出现 → 复活为 scanned）。
            let is_skip = existing.as_ref().is_some_and(|rec| {
                rec.size_bytes == size
                    && rec.mtime_ns == mtime_ns
                    && matches!(
                        rec.status.as_str(),
                        "scanned" | "duplicate_candidate" | "kept" | "failed"
                    )
            });
            if is_skip {
                summary.skipped += 1;
                report_progress!(summary, &path_str);
                continue;
            }

            let bytes = match std::fs::read(path) {
                Ok(b) => b,
                Err(e) => {
                    warn!(file_path = %path_str, error = %e, "读取文件失败");
                    summary.failed += 1;
                    report_progress!(summary, &path_str);
                    continue;
                }
            };
            let content_hash = format!("{:016x}", xxh3_64(&bytes));

            let file_name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path_str.clone());
            let parent_dir = path
                .parent()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default();

            // L2（A1）：内容哈希命中库中其他记录（在解析前判断——字节重复可直接删，省去解析）
            if existing.is_none() {
                if let Some(other) = db.find_by_content_hash(&content_hash)? {
                    if other.path != path_str {
                        if !Path::new(&other.path).exists() {
                            // 旧路径已消失 → 移动：复用记录，仅更新位置
                            db.update_file_location(
                                other.id,
                                &path_str,
                                &file_name,
                                &parent_dir,
                                size,
                                mtime_ns,
                            )?;
                            summary.updated += 1;
                            report_progress!(summary, &path_str);
                            continue;
                        }
                        if auto_dedup != AutoDedupMode::Off {
                            // 自动去重（字节相同）：直接硬删当前文件，不入库。
                            // 只以状态有效（scanned/candidate/kept）的记录为删除基准；
                            // 同路径已入库的文件由 existing 分支提前处理，永远不会走到这里被删。
                            if matches!(
                                other.status.as_str(),
                                "scanned" | "duplicate_candidate" | "kept"
                            ) {
                                match std::fs::remove_file(path) {
                                    Ok(()) => {
                                        summary.deleted_duplicates += 1;
                                    }
                                    // 已不存在（刚读到后又被删的竞态）→ 目标状态已达成，按成功计
                                    Err(e)
                                        if e.kind() == std::io::ErrorKind::NotFound =>
                                    {
                                        summary.deleted_duplicates += 1;
                                    }
                                    Err(e) => {
                                        warn!(file_path = %path_str, error = %e, "删除重复文件失败");
                                        summary.failed += 1;
                                    }
                                }
                                report_progress!(summary, &path_str);
                                continue;
                            }
                            // 基准状态无效（failed/missing）→ 照常入库，交由后续检测
                        }
                        // Off：旧路径仍存在 → 复制，照常入库，进入去重检测
                    }
                }
            }

            match midi_scan::extract_file_info(path) {
                Ok(info) => {
                    let note_total: i64 =
                        info.instruments.iter().map(|s| s.note_count as i64).sum();
                    // A2：无乐器文件不参与结构去重
                    let fingerprint = if info.instruments.is_empty() {
                        None
                    } else {
                        Some(duplicate::fingerprint(&info))
                    };

                    // 自动去重（字节相同）在解析前已处理；此处直接入库
                    let instruments: Vec<ScannedInstrumentInput> = info
                        .instruments
                        .iter()
                        .map(|s| ScannedInstrumentInput {
                            bank_msb: s.instrument.bank_msb,
                            bank_lsb: s.instrument.bank_lsb,
                            program: s.instrument.program,
                            is_percussion: s.instrument.is_percussion,
                            note_count: s.note_count,
                            name: s.instrument.display_name(),
                        })
                        .collect();
                    let file = ScannedFileInput {
                        path: path_str.clone(),
                        file_name,
                        parent_dir,
                        size_bytes: size,
                        mtime_ns,
                        content_hash,
                        fingerprint,
                        note_total,
                    };
                    let _id = db.save_scanned_file(existing.as_ref(), &file, &instruments)?;
                    if existing.is_none() {
                        summary.new += 1;
                    } else {
                        summary.updated += 1;
                    }
                    report_progress!(summary, &path_str);
                }
                Err(e) => {
                    warn!(file_path = %path_str, error = %e, "MIDI 解析失败");
                    match &existing {
                        Some(rec) => db.mark_file_failed(rec.id)?,
                        None => {
                            db.insert_failed_file(
                                &path_str,
                                &file_name,
                                &parent_dir,
                                size,
                                mtime_ns,
                            )?;
                        }
                    }
                    summary.failed += 1;
                    report_progress!(summary, &path_str);
                }
            }
        }

        // A4：对账排除 deleted/failed（reconcile_missing 内部已限定状态）
        let missing = db.reconcile_missing(&root_str, &present)?;
        summary.missing += missing;
    }

    // 全库全局去重检测（D7，一次性全量：扫描期已自动删除新重复，检测只处理历史遗留；
    // 完整扫描或中途取消都会执行）
    if run_dedup {
        let outcome = duplicate::detect_global_limit(db, usize::MAX)?;
        summary.duplicate_candidates = outcome.processed_files as u64;
    }

    info!(
        found = summary.found,
        new = summary.new,
        updated = summary.updated,
        skipped = summary.skipped,
        failed = summary.failed,
        deleted_duplicates = summary.deleted_duplicates,
        missing = summary.missing,
        "扫描完成"
    );
    Ok(summary)
}

fn is_midi_file(path: &Path) -> bool {
    path.extension()
        .map(|e| {
            let e = e.to_string_lossy().to_ascii_lowercase();
            e == "mid" || e == "midi"
        })
        .unwrap_or(false)
}

#[cfg(windows)]
fn is_hidden_or_system(path: &Path) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    const FILE_ATTRIBUTE_SYSTEM: u32 = 0x4;
    std::fs::metadata(path)
        .map(|m| (m.file_attributes() & (FILE_ATTRIBUTE_HIDDEN | FILE_ATTRIBUTE_SYSTEM)) != 0)
        .unwrap_or(false)
}

#[cfg(not(windows))]
fn is_hidden_or_system(path: &Path) -> bool {
    path.file_name()
        .map(|n| n.to_string_lossy().starts_with('.'))
        .unwrap_or(false)
}

fn mtime_nanos(meta: &std::fs::Metadata) -> i64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos() as i64)
        .unwrap_or(0)
}
