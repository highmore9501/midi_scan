//! 服务门面：CLI / TUI 统一入口。

use std::path::{Path, PathBuf};

use mm_db::{DbError, Repository};

use crate::duplicate::{self, DetectOutcome, DupGroup, ResolveOutcome};
use crate::query::{self, QueryParams, QueryRow};
use crate::scan::{self, ScanSummary};
use crate::CoreError;

pub struct Service {
    pub db: Repository,
}

#[derive(Debug, Clone)]
pub struct DbStats {
    pub counts: Vec<(String, i64)>,
    pub instrument_top: Vec<(String, i64)>,
}

impl Service {
    pub fn open(db_path: &Path) -> Result<Self, DbError> {
        Ok(Self {
            db: Repository::open(db_path)?,
        })
    }

    pub fn scan(
        &mut self,
        roots: &[PathBuf],
        run_dedup: bool,
        auto_dedup: scan::AutoDedupMode,
        progress: Option<&mut dyn FnMut(&scan::ScanProgress)>,
        cancel: Option<&std::sync::atomic::AtomicBool>,
    ) -> Result<ScanSummary, CoreError> {
        scan::scan_roots(&mut self.db, roots, run_dedup, auto_dedup, progress, cancel)
    }

    pub fn query(&self, params: &QueryParams) -> Result<Vec<QueryRow>, CoreError> {
        query::query_files(&self.db, params)
    }

    /// 全库去重检测（分批，单次最多 DETECT_BATCH_LIMIT 个候选文件）
    pub fn detect_duplicates(&mut self) -> Result<DetectOutcome, CoreError> {
        duplicate::detect_global_limit(&mut self.db, duplicate::DETECT_BATCH_LIMIT)
    }

    pub fn pending_groups(&self) -> Result<Vec<DupGroup>, CoreError> {
        let rows = self.db.pending_duplicate_groups()?;
        let mut groups = Vec::new();
        for row in rows {
            let members = self.db.group_members(row.id)?;
            groups.push(DupGroup {
                id: row.id,
                fingerprint: row.fingerprint,
                dup_type: row.dup_type,
                member_count: members.len(),
                members,
            });
        }
        Ok(groups)
    }

    pub fn resolve_group(
        &mut self,
        group_id: i64,
        keep_id: i64,
        deletes: &[i64],
    ) -> Result<ResolveOutcome, CoreError> {
        duplicate::resolve_group(&mut self.db, group_id, keep_id, deletes)
    }

    pub fn stats(&self) -> Result<DbStats, CoreError> {
        Ok(DbStats {
            counts: self.db.counts_by_status()?,
            instrument_top: self.db.instrument_top(20)?,
        })
    }
}
