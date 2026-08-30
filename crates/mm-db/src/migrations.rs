//! 建表与迁移：使用 `PRAGMA user_version` 维护版本号，启动时按版本顺序执行。

use rusqlite::Connection;

pub const SCHEMA_VERSION: i32 = 1;

/// 初始化/升级数据库（幂等）
pub fn migrate(conn: &Connection) -> rusqlite::Result<()> {
    let version: i32 = conn.query_row("PRAGMA user_version", [], |row| row.get(0))?;
    if version >= SCHEMA_VERSION {
        return Ok(());
    }
    conn.execute_batch(SCHEMA_SQL)?;
    conn.execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION}"))?;
    Ok(())
}

const SCHEMA_SQL: &str = r#"
-- 文件表：每条记录 = 一个已知的 MIDI 文件
CREATE TABLE IF NOT EXISTS files (
    id              INTEGER PRIMARY KEY,
    path            TEXT    NOT NULL UNIQUE,          -- 绝对路径（唯一）
    file_name       TEXT    NOT NULL,
    parent_dir      TEXT    NOT NULL,
    size_bytes      INTEGER NOT NULL,
    mtime_ns        INTEGER NOT NULL,                 -- 文件修改时间（ns）
    content_hash    TEXT,                             -- 字节内容哈希（强去重用）
    fingerprint     TEXT,                             -- 结构指纹（弱去重用）
    note_total      INTEGER NOT NULL DEFAULT 0,       -- 全文件总音符数（冗余，便于排序）
    status          TEXT    NOT NULL DEFAULT 'scanned',
                      -- scanned / duplicate_candidate / kept / deleted / missing / failed
    first_scanned_at  INTEGER NOT NULL,               -- unix 秒
    last_verified_at  INTEGER,
    deleted_at        INTEGER,
    deleted_reason    TEXT
);
CREATE INDEX IF NOT EXISTS idx_files_status ON files(status);
CREATE INDEX IF NOT EXISTS idx_files_parent ON files(parent_dir);
CREATE INDEX IF NOT EXISTS idx_files_fp     ON files(fingerprint);
CREATE INDEX IF NOT EXISTS idx_files_hash   ON files(content_hash);

-- 乐器表：全库共享的乐器字典（GM 128 + 打击乐 + 扩展）
CREATE TABLE IF NOT EXISTS instruments (
    id            INTEGER PRIMARY KEY,
    bank_msb      INTEGER NOT NULL DEFAULT 0,
    bank_lsb      INTEGER NOT NULL DEFAULT 0,
    program       INTEGER NOT NULL,
    is_percussion INTEGER NOT NULL DEFAULT 0,
    name          TEXT    NOT NULL,                  -- 显示名（来自 GM 表/扩展表）
    UNIQUE(bank_msb, bank_lsb, program, is_percussion)
);

-- 文件 × 乐器 关联表：核心明细，支持组合查询与去重指纹
CREATE TABLE IF NOT EXISTS file_instruments (
    file_id       INTEGER NOT NULL REFERENCES files(id)      ON DELETE CASCADE,
    instrument_id INTEGER NOT NULL REFERENCES instruments(id),
    note_count    INTEGER NOT NULL,
    PRIMARY KEY (file_id, instrument_id)
);
CREATE INDEX IF NOT EXISTS idx_fi_instrument ON file_instruments(instrument_id);

-- 扫描批次：增量扫描的审计与恢复依据
CREATE TABLE IF NOT EXISTS scan_batches (
    id             INTEGER PRIMARY KEY,
    root_dir       TEXT    NOT NULL,
    started_at     INTEGER NOT NULL,
    finished_at    INTEGER,
    files_found    INTEGER NOT NULL DEFAULT 0,
    files_new      INTEGER NOT NULL DEFAULT 0,
    files_updated  INTEGER NOT NULL DEFAULT 0,
    files_skipped  INTEGER NOT NULL DEFAULT 0,
    files_failed   INTEGER NOT NULL DEFAULT 0,
    files_missing  INTEGER NOT NULL DEFAULT 0
);

-- 去重候选组：一组「互为重复」的文件，等待用户确认；
-- fingerprint 无唯一约束：组 resolved/dismissed 后再次发现同指纹重复时新建 pending 组
CREATE TABLE IF NOT EXISTS duplicate_groups (
    id             INTEGER PRIMARY KEY,
    fingerprint    TEXT NOT NULL,
    dup_type       TEXT NOT NULL,                -- byte_identical / structurally_identical
    status         TEXT NOT NULL DEFAULT 'pending',  -- pending / resolved / dismissed
    created_at     INTEGER NOT NULL,
    resolved_at    INTEGER
);
CREATE INDEX IF NOT EXISTS idx_dup_groups_fp ON duplicate_groups(fingerprint);

CREATE TABLE IF NOT EXISTS duplicate_group_members (
    group_id  INTEGER NOT NULL REFERENCES duplicate_groups(id) ON DELETE CASCADE,
    file_id   INTEGER NOT NULL REFERENCES files(id)          ON DELETE CASCADE,
    PRIMARY KEY (group_id, file_id)
);
"#;
