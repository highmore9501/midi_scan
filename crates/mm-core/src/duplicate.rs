//! 去重检测与用户确认（F5）：结构指纹 + 字节指纹，全库全局（D7），默认硬删（D10）。

use std::collections::HashSet;

use mm_db::{FileRecord, Repository};
use midi_scan::MidiFileInfo;
use xxhash_rust::xxh3::xxh3_64;

use crate::CoreError;

/// 结构指纹（ADR-3）：按乐器 key 排序的 (乐器, 音符数) 列表哈希。
/// 排序保证与乐器出现顺序无关；调用方需保证传入的文件有乐器（A2）。
pub fn fingerprint(info: &MidiFileInfo) -> String {
    let mut parts: Vec<String> = info
        .instruments
        .iter()
        .map(|s| {
            format!(
                "{:02x}{:02x}{:02x}{}:{}",
                s.instrument.bank_msb,
                s.instrument.bank_lsb,
                s.instrument.program,
                if s.instrument.is_percussion { 1 } else { 0 },
                s.note_count
            )
        })
        .collect();
    parts.sort();
    let joined = parts.join("|");
    format!("{:016x}", xxh3_64(joined.as_bytes()))
}

#[derive(Debug, Clone)]
pub struct DupGroup {
    pub id: i64,
    pub fingerprint: String,
    pub dup_type: String, // byte_identical / structurally_identical
    pub members: Vec<FileRecord>,
    pub member_count: usize,
}

/// 单次去重检测的候选文件数上限：一次最多处理这么多候选文件，剩余留待下次检测
pub const DETECT_BATCH_LIMIT: usize = 1000;

/// 单次去重检测的结果
#[derive(Debug, Clone)]
pub struct DetectOutcome {
    /// 本次处理（新建/更新）的候选组数
    pub processed_groups: usize,
    /// 本次标记为候选的文件数
    pub processed_files: usize,
    /// 仍未检测（还没有 pending 组）的重复指纹组数；处理完当前批次后需再次检测
    pub remaining_groups: usize,
}

/// 全库全局去重检测（D7），**分批执行**：
/// - 每次最多处理 `max_files` 个候选文件，达到上限即停止，剩余组留待下次检测；
/// - 已有 pending 组的哈希跳过（已检测过，避免重复处理）；
/// - **v0.11 起仅按字节完全相同（content_hash 一致）判定重复**，不再把"结构相同"算作重复；
/// - 组内所有文件（含存量）→ status='duplicate_candidate'（批量事务写入）；
/// - duplicate_groups.fingerprint 列存放内容哈希（无唯一约束 A3：同哈希可再次建 pending 组）。
pub fn detect_global_limit(
    db: &mut Repository,
    max_files: usize,
) -> Result<DetectOutcome, CoreError> {
    let cancel = std::sync::atomic::AtomicBool::new(false);
    detect_streaming(db, max_files, &cancel, |_| Ok(()))
}

/// 流式去重检测：逐内容哈希处理，每建一个候选组立即通过 `on_group` 回调交给调用方
/// （前端可边收边处理删除）；`cancel` 置 true 时提前停止。
/// 逐哈希独立查询 → 每次读到最新提交，已删除/已解决的文件自动被过滤，与用户删除操作并发安全（WAL）。
pub fn detect_streaming<F>(
    db: &mut Repository,
    max_files: usize,
    cancel: &std::sync::atomic::AtomicBool,
    mut on_group: F,
) -> Result<DetectOutcome, CoreError>
where
    F: FnMut(&DupGroup) -> Result<(), CoreError>,
{
    let hashes = db.duplicate_content_hashes()?;
    // 已存在 pending 组的哈希：跳过（已检测过，避免重复处理）
    let pending = db.pending_group_hashes()?;

    let mut processed_groups = 0usize;
    let mut processed_files = 0usize;
    let mut new_member_ids: Vec<i64> = Vec::new();

    for h in &hashes {
        if cancel.load(std::sync::atomic::Ordering::Relaxed) {
            break;
        }
        if pending.contains(h) {
            continue;
        }
        let members = db.files_by_content_hash(h)?;
        if members.len() < 2 {
            continue;
        }
        // 达到单次上限：停止，剩余组留待下次检测
        if processed_files + members.len() > max_files {
            break;
        }
        // 同内容哈希 → 字节完全相同（v0.11 起去重只认字节相同）
        let dup_type = "byte_identical";
        let group_id = db.insert_duplicate_group(h, dup_type)?;
        let member_ids: Vec<i64> = members.iter().map(|m| m.id).collect();
        db.replace_group_members(group_id, &member_ids)?;
        new_member_ids.extend(member_ids.iter().copied());
        let group = DupGroup {
            id: group_id,
            fingerprint: h.clone(),
            dup_type: dup_type.to_string(),
            member_count: member_ids.len(),
            members,
        };
        on_group(&group)?;
        processed_groups += 1;
        processed_files += member_ids.len();
    }

    // 批量标记候选（单事务，避免逐条提交导致卡顿）；
    // 条件更新：不把已删/缺失/失败的文件改回候选（与用户删除操作并发安全）
    if !new_member_ids.is_empty() {
        db.set_files_status_batch(&new_member_ids, "duplicate_candidate")?;
    }
    // 已不再重复的 pending 组标记 dismissed（避免陈旧候选组一直挂着）
    db.dismiss_stale_groups()?;
    // 不再属于任何 pending 组的候选文件恢复 scanned
    db.reset_stale_candidates()?;

    let remaining_groups = db.remaining_duplicate_hashes()? as usize;
    Ok(DetectOutcome {
        processed_groups,
        processed_files,
        remaining_groups,
    })
}

#[derive(Debug, Clone)]
pub struct ResolveOutcome {
    pub deleted: u64,
}

/// 用户确认后执行：保留 `keep_id`，硬删 `deletes`（D10，调用方需先经过 UI 确认）。
pub fn resolve_group(
    db: &mut Repository,
    group_id: i64,
    keep_id: i64,
    deletes: &[i64],
) -> Result<ResolveOutcome, CoreError> {
    let members = db.group_members(group_id)?;
    let member_ids: HashSet<i64> = members.iter().map(|m| m.id).collect();
    if !member_ids.contains(&keep_id) {
        return Err(CoreError::Other(format!("保留文件 {keep_id} 不在该去重组内")));
    }
    for id in deletes {
        if !member_ids.contains(id) {
            return Err(CoreError::Other(format!("文件 {id} 不在该去重组内")));
        }
    }

    let mut deleted = 0u64;
    for id in deletes {
        let rec = members.iter().find(|m| m.id == *id).expect("成员已校验");
        match std::fs::remove_file(&rec.path) {
            Ok(()) => {
                let now = Repository::now_secs();
                db.set_file_status(*id, "deleted")?;
                db.conn().execute(
                    "UPDATE files SET deleted_at=?1, deleted_reason='dedup_hard' WHERE id=?2",
                    rusqlite::params![now, id],
                )?;
                deleted += 1;
            }
            Err(e) => {
                return Err(CoreError::Other(format!("删除失败 {}: {}", rec.path, e)));
            }
        }
    }
    db.set_file_status(keep_id, "kept")?;
    db.mark_group_resolved(group_id)?;
    Ok(ResolveOutcome { deleted })
}
