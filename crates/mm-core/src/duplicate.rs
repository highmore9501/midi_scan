//! 去重检测与用户确认（F5）：结构指纹 + 字节指纹，全库全局（D7），默认硬删（D10）。

use std::collections::HashSet;

use midi_scan::MidiFileInfo;
use mm_db::{FileRecord, Repository};
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

/// 单次去重检测的结果
#[derive(Debug, Clone)]
pub struct DetectOutcome {
    /// 本次处理（新建/更新）的候选组数
    pub processed_groups: usize,
    /// 本次标记为候选的文件数
    pub processed_files: usize,
    /// 仍未检测（还没有 pending 组）的重复哈希组数（一次性全量下通常为 0；仅被停止时非 0）
    pub remaining_groups: usize,
}

/// 全库全局去重检测（D7）。`max_files` 为单次候选文件数上限，
/// **v0.12 起调用方传 `usize::MAX` 表示一次性全量检测**（扫描期已自动删除新重复，
/// 检测只处理历史遗留，跑一次即可；结果全部保存为候选组，由用户分批去重清理）：
/// - 已有 pending 组的哈希跳过（已检测过，避免重复处理）；
/// - **仅按字节完全相同（content_hash 一致）判定重复**，不再把"结构相同"算作重复；
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
/// 性能（v0.13）：所有数据库状态变更合并为**单事务一次提交**（旧实现每个文件 2 次自动提交，
/// 大量删除时 fsync 开销明显）；SQL 保持每条短语句，不拼长 IN 列表，避免变量数上限问题。
/// 容错（v0.13）：文件已不存在（NotFound）视为删除成功——陈旧记录不再阻塞候选组。
pub fn resolve_group(
    db: &mut Repository,
    group_id: i64,
    keep_id: i64,
    deletes: &[i64],
) -> Result<ResolveOutcome, CoreError> {
    let members = db.group_members(group_id)?;
    let member_ids: HashSet<i64> = members.iter().map(|m| m.id).collect();
    if !member_ids.contains(&keep_id) {
        return Err(CoreError::Other(format!(
            "保留文件 {keep_id} 不在该去重组内"
        )));
    }
    for id in deletes {
        if !member_ids.contains(id) {
            return Err(CoreError::Other(format!("文件 {id} 不在该去重组内")));
        }
    }

    // 1) 先逐个物理硬删；「文件已不存在」（os error 2）视为成功——目标状态已达成，
    //    常见于库中有陈旧记录（文件被外部删除/上次中断残留），照常标记删除并继续；
    //    其余错误（权限、被占用等）才是真失败：即停，已删部分照常入库，组保持 pending。
    let mut deleted_ids: Vec<i64> = Vec::with_capacity(deletes.len());
    let mut first_err: Option<CoreError> = None;
    for id in deletes {
        let rec = members.iter().find(|m| m.id == *id).expect("成员已校验");
        match std::fs::remove_file(&rec.path) {
            Ok(()) => deleted_ids.push(*id),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => deleted_ids.push(*id),
            Err(e) => {
                first_err = Some(CoreError::Other(format!("删除失败 {}: {}", rec.path, e)));
                break;
            }
        }
    }

    // 2) 单事务写入：状态 + deleted_at + 原因合并为一条短 UPDATE，整组一次提交。
    let now = Repository::now_secs();
    let tx = db.conn_mut().transaction()?;
    for id in &deleted_ids {
        tx.execute(
            "UPDATE files SET status='deleted', deleted_at=?1, deleted_reason='dedup_hard' WHERE id=?2",
            rusqlite::params![now, id],
        )?;
    }
    if first_err.is_none() {
        tx.execute(
            "UPDATE files SET status='kept' WHERE id=?1",
            rusqlite::params![keep_id],
        )?;
        tx.execute(
            "UPDATE duplicate_groups SET status='resolved', resolved_at=?1 WHERE id=?2",
            rusqlite::params![now, group_id],
        )?;
    }
    tx.commit()?;

    if let Some(e) = first_err {
        return Err(e);
    }
    Ok(ResolveOutcome {
        deleted: deleted_ids.len() as u64,
    })
}
