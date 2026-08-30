//! Repository：SQLite 读写封装，写入路径统一收敛到这里，避免竞争。

use std::collections::HashSet;
use std::path::Path;

use rusqlite::{params, Connection};

use crate::migrations;

#[derive(Debug, thiserror::Error)]
pub enum DbError {
    #[error("SQLite 错误: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("IO 错误: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Clone)]
pub struct FileRecord {
    pub id: i64,
    pub path: String,
    pub file_name: String,
    pub parent_dir: String,
    pub size_bytes: i64,
    pub mtime_ns: i64,
    pub content_hash: Option<String>,
    pub fingerprint: Option<String>,
    pub note_total: i64,
    pub status: String,
    pub first_scanned_at: i64,
}

/// 扫描成功文件的入库输入
#[derive(Debug, Clone)]
pub struct ScannedFileInput {
    pub path: String,
    pub file_name: String,
    pub parent_dir: String,
    pub size_bytes: i64,
    pub mtime_ns: i64,
    pub content_hash: String,
    pub fingerprint: Option<String>,
    pub note_total: i64,
}

/// 单个乐器的入库输入
#[derive(Debug, Clone)]
pub struct ScannedInstrumentInput {
    pub bank_msb: u8,
    pub bank_lsb: u8,
    pub program: u8,
    pub is_percussion: bool,
    pub note_count: u64,
    pub name: String,
}

#[derive(Debug, Clone)]
pub struct DupGroupRow {
    pub id: i64,
    pub fingerprint: String,
    pub dup_type: String,
    pub created_at: i64,
}

pub struct Repository {
    conn: Connection,
}

impl Repository {
    pub fn open(db_path: &Path) -> Result<Self, DbError> {
        if let Some(parent) = db_path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let conn = Connection::open(db_path)?;
        // WAL：允许 TUI 读与后台扫描写并发
        let _mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.pragma_update(None, "busy_timeout", 10_000)?;
        migrations::migrate(&conn)?;
        Ok(Self { conn })
    }

    pub fn now_secs() -> i64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0)
    }

    pub fn conn(&self) -> &Connection {
        &self.conn
    }

    // ---------- 文件查询 ----------

    pub fn find_by_path(&self, path: &str) -> Result<Option<FileRecord>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, file_name, parent_dir, size_bytes, mtime_ns, content_hash, fingerprint, note_total, status, first_scanned_at
             FROM files WHERE path = ?1",
        )?;
        let mut rows = stmt.query_map(params![path], row_to_file)?;
        Ok(rows.next().transpose()?)
    }

    pub fn find_by_content_hash(&self, hash: &str) -> Result<Option<FileRecord>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, file_name, parent_dir, size_bytes, mtime_ns, content_hash, fingerprint, note_total, status, first_scanned_at
             FROM files WHERE content_hash = ?1 AND status != 'deleted' LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![hash], row_to_file)?;
        Ok(rows.next().transpose()?)
    }

    /// 同一内容哈希的全部有效文件（字节重复分组；v0.11 起去重只认字节相同）
    pub fn files_by_content_hash(&self, hash: &str) -> Result<Vec<FileRecord>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, path, file_name, parent_dir, size_bytes, mtime_ns, content_hash, fingerprint, note_total, status, first_scanned_at
             FROM files WHERE content_hash = ?1 AND status NOT IN ('deleted','missing','failed') ORDER BY id",
        )?;
        let rows = stmt.query_map(params![hash], row_to_file)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// 内容哈希重复（候选组）的哈希列表：状态有效、至少 2 个文件（v0.11 起仅按字节相同判定重复）
    pub fn duplicate_content_hashes(&self) -> Result<Vec<String>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT content_hash FROM files
             WHERE status NOT IN ('deleted','missing','failed')
               AND content_hash IS NOT NULL
             GROUP BY content_hash HAVING COUNT(*) > 1",
        )?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    // ---------- 扫描写入 ----------

    /// 保存一次成功扫描的结果（文件行 upsert + 乐器关联整体替换），单个事务。
    pub fn save_scanned_file(
        &mut self,
        existing: Option<&FileRecord>,
        file: &ScannedFileInput,
        instruments: &[ScannedInstrumentInput],
    ) -> Result<i64, DbError> {
        let now = Self::now_secs();
        let tx = self.conn.transaction()?;
        let id = match existing {
            Some(rec) => {
                tx.execute(
                    "UPDATE files SET path=?1, file_name=?2, parent_dir=?3, size_bytes=?4, mtime_ns=?5,
                     content_hash=?6, fingerprint=?7, note_total=?8, status='scanned',
                     last_verified_at=?9, deleted_at=NULL, deleted_reason=NULL WHERE id=?10",
                    params![
                        file.path,
                        file.file_name,
                        file.parent_dir,
                        file.size_bytes,
                        file.mtime_ns,
                        file.content_hash,
                        file.fingerprint.as_deref(),
                        file.note_total,
                        now,
                        rec.id
                    ],
                )?;
                rec.id
            }
            None => {
                tx.execute(
                    "INSERT INTO files (path, file_name, parent_dir, size_bytes, mtime_ns, content_hash, fingerprint, note_total, status, first_scanned_at, last_verified_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,'scanned',?9,?9)",
                    params![
                        file.path,
                        file.file_name,
                        file.parent_dir,
                        file.size_bytes,
                        file.mtime_ns,
                        file.content_hash,
                        file.fingerprint.as_deref(),
                        file.note_total,
                        now
                    ],
                )?;
                tx.last_insert_rowid()
            }
        };

        tx.execute("DELETE FROM file_instruments WHERE file_id=?1", params![id])?;
        for ins in instruments {
            let ins_id = get_or_create_instrument(&tx, ins)?;
            tx.execute(
                "INSERT INTO file_instruments (file_id, instrument_id, note_count) VALUES (?1,?2,?3)",
                params![id, ins_id, ins.note_count as i64],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// 移动场景：复用已有记录，仅更新路径与元数据
    pub fn update_file_location(
        &mut self,
        id: i64,
        path: &str,
        file_name: &str,
        parent_dir: &str,
        size_bytes: i64,
        mtime_ns: i64,
    ) -> Result<(), DbError> {
        let now = Self::now_secs();
        self.conn.execute(
            "UPDATE files SET path=?1, file_name=?2, parent_dir=?3, size_bytes=?4, mtime_ns=?5,
             status='scanned', last_verified_at=?6, deleted_at=NULL, deleted_reason=NULL WHERE id=?7",
            params![path, file_name, parent_dir, size_bytes, mtime_ns, now, id],
        )?;
        Ok(())
    }

    /// 解析失败：标记已有记录为 failed（保留明细，避免反复重试；可手动重扫）
    pub fn mark_file_failed(&mut self, id: i64) -> Result<(), DbError> {
        let now = Self::now_secs();
        self.conn.execute(
            "UPDATE files SET status='failed', last_verified_at=?1 WHERE id=?2",
            params![now, id],
        )?;
        Ok(())
    }

    /// 解析失败：插入最小记录（status='failed'），避免每次扫描都重试损坏文件
    pub fn insert_failed_file(
        &mut self,
        path: &str,
        file_name: &str,
        parent_dir: &str,
        size_bytes: i64,
        mtime_ns: i64,
    ) -> Result<i64, DbError> {
        let now = Self::now_secs();
        self.conn.execute(
            "INSERT INTO files (path, file_name, parent_dir, size_bytes, mtime_ns, content_hash, fingerprint, note_total, status, first_scanned_at, last_verified_at)
             VALUES (?1,?2,?3,?4,?5,NULL,NULL,0,'failed',?6,?6)",
            params![path, file_name, parent_dir, size_bytes, mtime_ns, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    /// 快照对账：root 下已入库但本次扫描未出现的文件 → status='missing'（排除 deleted/failed）
    pub fn reconcile_missing(&mut self, root: &str, present: &HashSet<String>) -> Result<u64, DbError> {
        let like = format!("{root}%");
        let mut stmt = self.conn.prepare(
            "SELECT id, path FROM files WHERE status IN ('scanned','duplicate_candidate','kept') AND path LIKE ?1",
        )?;
        let rows = stmt.query_map(params![like], |row| {
            Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut ids = Vec::new();
        for r in rows {
            let (id, p) = r?;
            let under_root = p == root
                || p.starts_with(&format!("{root}\\"))
                || p.starts_with(&format!("{root}/"));
            if under_root && !present.contains(&p) {
                ids.push(id);
            }
        }
        let mut count = 0u64;
        for id in ids {
            self.conn.execute("UPDATE files SET status='missing' WHERE id=?1", params![id])?;
            count += 1;
        }
        Ok(count)
    }

    // ---------- 去重组 ----------

    pub fn pending_duplicate_groups(&self) -> Result<Vec<DupGroupRow>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, fingerprint, dup_type, created_at FROM duplicate_groups WHERE status='pending' ORDER BY id",
        )?;
        let rows = stmt.query_map([], dup_group_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn find_pending_group_by_fingerprint(&self, fp: &str) -> Result<Option<DupGroupRow>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, fingerprint, dup_type, created_at FROM duplicate_groups WHERE fingerprint=?1 AND status='pending' LIMIT 1",
        )?;
        let mut rows = stmt.query_map(params![fp], dup_group_row)?;
        Ok(rows.next().transpose()?)
    }

    pub fn insert_duplicate_group(&mut self, fp: &str, dup_type: &str) -> Result<i64, DbError> {
        let now = Self::now_secs();
        self.conn.execute(
            "INSERT INTO duplicate_groups (fingerprint, dup_type, status, created_at) VALUES (?1,?2,'pending',?3)",
            params![fp, dup_type, now],
        )?;
        Ok(self.conn.last_insert_rowid())
    }

    pub fn replace_group_members(&mut self, group_id: i64, member_ids: &[i64]) -> Result<(), DbError> {
        let tx = self.conn.transaction()?;
        tx.execute("DELETE FROM duplicate_group_members WHERE group_id=?1", params![group_id])?;
        for &id in member_ids {
            tx.execute(
                "INSERT OR IGNORE INTO duplicate_group_members (group_id, file_id) VALUES (?1,?2)",
                params![group_id, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn group_members(&self, group_id: i64) -> Result<Vec<FileRecord>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT f.id, f.path, f.file_name, f.parent_dir, f.size_bytes, f.mtime_ns, f.content_hash, f.fingerprint, f.note_total, f.status, f.first_scanned_at
             FROM duplicate_group_members m JOIN files f ON f.id=m.file_id
             WHERE m.group_id=?1 ORDER BY f.id",
        )?;
        let rows = stmt.query_map(params![group_id], row_to_file)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    pub fn set_file_status(&mut self, id: i64, status: &str) -> Result<(), DbError> {
        self.conn.execute("UPDATE files SET status=?1 WHERE id=?2", params![status, id])?;
        Ok(())
    }

    pub fn mark_group_resolved(&mut self, group_id: i64) -> Result<(), DbError> {
        let now = Self::now_secs();
        self.conn.execute(
            "UPDATE duplicate_groups SET status='resolved', resolved_at=?1 WHERE id=?2",
            params![now, group_id],
        )?;
        Ok(())
    }

    /// 清空失效候选：已不在任何 pending 组中的 duplicate_candidate 文件恢复为 scanned
    pub fn reset_stale_candidates(&mut self) -> Result<u64, DbError> {
        let n = self.conn.execute(
            "UPDATE files SET status='scanned' WHERE status='duplicate_candidate' AND id NOT IN (
                SELECT m.file_id FROM duplicate_group_members m
                JOIN duplicate_groups g ON g.id=m.group_id WHERE g.status='pending')",
            [],
        )?;
        Ok(n as u64)
    }

    /// 将所有 pending 去重组标记为 dismissed（清空候选；不删除任何文件，调用方随后应 reset_stale_candidates 恢复文件状态）
    pub fn dismiss_all_pending_groups(&mut self) -> Result<u64, DbError> {
        let n = self.conn.execute(
            "UPDATE duplicate_groups SET status='dismissed', resolved_at=?1 WHERE status='pending'",
            params![Self::now_secs()],
        )?;
        Ok(n as u64)
    }

    /// 标记已不再重复的 pending 组为 dismissed（不再打扰）。
    /// 用子查询判定"当前仍重复的内容哈希"，避免长 IN 列表超出 SQLite 变量上限（默认 999）。
    /// 注：duplicate_groups.fingerprint 列自 v0.11 起存放内容哈希。
    pub fn dismiss_stale_groups(&mut self) -> Result<u64, DbError> {
        let n = self.conn.execute(
            "UPDATE duplicate_groups SET status='dismissed'
             WHERE status='pending' AND fingerprint NOT IN (
                 SELECT content_hash FROM files
                 WHERE status NOT IN ('deleted','missing','failed')
                   AND content_hash IS NOT NULL
                 GROUP BY content_hash HAVING COUNT(*) > 1
             )",
            [],
        )?;
        Ok(n as u64)
    }

    /// 已有 pending 组的哈希集合（分批检测时跳过已检测的哈希，避免重复处理）
    pub fn pending_group_hashes(&self) -> Result<HashSet<String>, DbError> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT fingerprint FROM duplicate_groups WHERE status='pending'")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        Ok(rows.collect::<Result<HashSet<_>, _>>()?)
    }

    /// 剩余待检测的重复哈希组数：还没有 pending 组、且当前仍重复（COUNT(*) > 1）的内容哈希数
    pub fn remaining_duplicate_hashes(&self) -> Result<u64, DbError> {
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM (
                SELECT f.content_hash FROM files f
                WHERE f.status NOT IN ('deleted','missing','failed')
                  AND f.content_hash IS NOT NULL
                  AND f.content_hash NOT IN (SELECT fingerprint FROM duplicate_groups WHERE status='pending')
                GROUP BY f.content_hash HAVING COUNT(*) > 1
            )",
            [],
            |row| row.get(0),
        )?;
        Ok(n as u64)
    }

    /// 批量更新文件状态（单事务，避免逐条自动提交导致性能问题）。
    /// 条件更新：不把已删除/缺失/失败的文件改回候选（与用户删除操作并发安全）。
    pub fn set_files_status_batch(&mut self, ids: &[i64], status: &str) -> Result<(), DbError> {
        let tx = self.conn.transaction()?;
        for id in ids {
            tx.execute(
                "UPDATE files SET status=?1 WHERE id=?2 AND status NOT IN ('deleted','missing','failed')",
                params![status, id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ---------- 统计 ----------

    pub fn counts_by_status(&self) -> Result<Vec<(String, i64)>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT status, COUNT(*) FROM files GROUP BY status ORDER BY status",
        )?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// 乐器分布 Top N（按使用该乐器的文件数；**排除打击乐**，打击乐按音高细分会刷屏）
    pub fn instrument_top(&self, limit: i64) -> Result<Vec<(String, i64)>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT i.name, COUNT(DISTINCT fi.file_id) AS cnt
             FROM file_instruments fi
             JOIN instruments i ON i.id = fi.instrument_id
             JOIN files f ON f.id = fi.file_id
             WHERE f.status NOT IN ('deleted','missing','failed')
               AND i.is_percussion = 0
             GROUP BY i.id ORDER BY cnt DESC, i.name LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// 库里出现过的乐器（供查询界面多选）：(id, bank_msb, bank_lsb, program, is_percussion, name)
    pub fn list_instruments(&self) -> Result<Vec<(i64, i64, i64, i64, i64, String)>, DbError> {
        let mut stmt = self.conn.prepare(
            "SELECT id, bank_msb, bank_lsb, program, is_percussion, name FROM instruments ORDER BY name, program",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, i64>(2)?,
                row.get::<_, i64>(3)?,
                row.get::<_, i64>(4)?,
                row.get::<_, String>(5)?,
            ))
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }
}

fn row_to_file(row: &rusqlite::Row<'_>) -> rusqlite::Result<FileRecord> {
    Ok(FileRecord {
        id: row.get(0)?,
        path: row.get(1)?,
        file_name: row.get(2)?,
        parent_dir: row.get(3)?,
        size_bytes: row.get(4)?,
        mtime_ns: row.get(5)?,
        content_hash: row.get(6)?,
        fingerprint: row.get(7)?,
        note_total: row.get(8)?,
        status: row.get(9)?,
        first_scanned_at: row.get(10)?,
    })
}

fn dup_group_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<DupGroupRow> {
    Ok(DupGroupRow {
        id: row.get(0)?,
        fingerprint: row.get(1)?,
        dup_type: row.get(2)?,
        created_at: row.get(3)?,
    })
}

fn get_or_create_instrument(
    tx: &rusqlite::Transaction<'_>,
    ins: &ScannedInstrumentInput,
) -> Result<i64, DbError> {
    tx.execute(
        "INSERT OR IGNORE INTO instruments (bank_msb, bank_lsb, program, is_percussion, name) VALUES (?1,?2,?3,?4,?5)",
        params![
            ins.bank_msb as i64,
            ins.bank_lsb as i64,
            ins.program as i64,
            ins.is_percussion as i64,
            ins.name
        ],
    )?;
    let id: i64 = tx.query_row(
        "SELECT id FROM instruments WHERE bank_msb=?1 AND bank_lsb=?2 AND program=?3 AND is_percussion=?4",
        params![
            ins.bank_msb as i64,
            ins.bank_lsb as i64,
            ins.program as i64,
            ins.is_percussion as i64
        ],
        |row| row.get(0),
    )?;
    Ok(id)
}
