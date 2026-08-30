//! 查询引擎：Exact 组合查询（D6）——文件的乐器集合恰好等于所选集合，
//! 支持每乐器音符数区间、总音符数区间、文件名关键词、目录前缀过滤与分页。

use std::collections::HashMap;

use midi_scan::InstrumentId;
use mm_db::Repository;

use crate::CoreError;

/// 乐器集合匹配模式
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MatchMode {
    /// 精确匹配：文件乐器集合恰好等于所选集合
    #[default]
    Exact,
    /// 包含匹配：文件包含所选的全部乐器，允许包含其他乐器
    Superset,
}

#[derive(Debug, Clone, Default)]
pub struct QueryParams {
    /// 用户勾选的乐器（空 = 不限）
    pub selected: Vec<InstrumentId>,
    /// 匹配模式（默认 Exact）
    pub match_mode: MatchMode,
    /// 每个所选乐器的音符数区间（key = InstrumentId）
    pub per_instrument_note_range: HashMap<InstrumentId, (u64, u64)>,
    /// 文件总音符数区间（可选）
    pub total_note_range: Option<(u64, u64)>,
    /// 文件名模糊匹配（可选）
    pub name_keyword: Option<String>,
    /// 限定目录前缀（可选）
    pub dir_filter: Option<String>,
    pub limit: u64,
    pub offset: u64,
}

#[derive(Debug, Clone)]
pub struct QueryRow {
    pub id: i64,
    pub path: String,
    pub file_name: String,
    pub note_total: i64,
    /// (乐器名, 音符数)
    pub instruments: Vec<(String, i64)>,
}

/// Exact 组合查询（D6）
pub fn query_files(db: &Repository, p: &QueryParams) -> Result<Vec<QueryRow>, CoreError> {
    let mut sql = String::from(
        "SELECT f.id, f.path, f.file_name, f.note_total
         FROM files f
         JOIN file_instruments fi ON fi.file_id = f.id
         WHERE f.status NOT IN ('deleted','missing','failed')",
    );
    let mut args: Vec<Box<dyn rusqlite::ToSql>> = Vec::new();

    if let Some(kw) = &p.name_keyword {
        if !kw.is_empty() {
            sql.push_str(" AND f.file_name LIKE ?");
            args.push(Box::new(format!("%{kw}%")));
        }
    }
    if let Some(dir) = &p.dir_filter {
        if !dir.is_empty() {
            sql.push_str(" AND f.parent_dir LIKE ?");
            args.push(Box::new(format!("{dir}%")));
        }
    }
    if let Some((lo, hi)) = p.total_note_range {
        sql.push_str(" AND f.note_total >= ? AND f.note_total <= ?");
        args.push(Box::new(lo as i64));
        args.push(Box::new(hi as i64));
    }

    sql.push_str(" GROUP BY f.id");

    // 乐器集合匹配（Exact / Superset）
    if !p.selected.is_empty() {
        let mut sel_ids: Vec<i64> = Vec::new();
        for inst in &p.selected {
            match resolve_instrument_db_id(db, inst)? {
                Some(id) => sel_ids.push(id),
                // 所选乐器在库里不存在 → 不可能有匹配文件
                None => return Ok(Vec::new()),
            }
        }

        let n = sel_ids.len();
        let ph = placeholders(n);
        sql.push_str(" HAVING");
        match p.match_mode {
            MatchMode::Exact => {
                // 乐器集合恰好等于所选集合
                sql.push_str(&format!(" COUNT(DISTINCT fi.instrument_id) = {n}"));
                sql.push_str(&format!(
                    " AND COUNT(DISTINCT CASE WHEN fi.instrument_id NOT IN ({ph}) THEN fi.instrument_id END) = 0"
                ));
            }
            MatchMode::Superset => {
                // 包含所选全部乐器（允许更多）
                sql.push_str(&format!(
                    " COUNT(DISTINCT CASE WHEN fi.instrument_id IN ({ph}) THEN fi.instrument_id END) = {n}"
                ));
            }
        }
        for id in &sel_ids {
            args.push(Box::new(*id));
        }

        // 每乐器音符数区间
        for (idx, inst) in p.selected.iter().enumerate() {
            if let Some((lo, hi)) = p.per_instrument_note_range.get(inst) {
                sql.push_str(
                    " AND MAX(CASE WHEN fi.instrument_id=? THEN fi.note_count END) >= ?",
                );
                args.push(Box::new(sel_ids[idx]));
                args.push(Box::new(*lo as i64));
                sql.push_str(
                    " AND MAX(CASE WHEN fi.instrument_id=? THEN fi.note_count END) <= ?",
                );
                args.push(Box::new(sel_ids[idx]));
                args.push(Box::new(*hi as i64));
            }
        }
    }

    sql.push_str(" ORDER BY f.note_total DESC, f.file_name LIMIT ? OFFSET ?");
    args.push(Box::new(p.limit as i64));
    args.push(Box::new(p.offset as i64));

    let mut stmt = db.conn().prepare(&sql)?;
    let sql_args: Vec<&dyn rusqlite::ToSql> = args.iter().map(|b| b.as_ref()).collect();
    let rows = stmt.query_map(rusqlite::params_from_iter(sql_args), |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
        ))
    })?;

    let mut out: Vec<QueryRow> = Vec::new();
    let mut file_ids: Vec<i64> = Vec::new();
    for r in rows {
        let (id, path, file_name, note_total) = r?;
        file_ids.push(id);
        out.push(QueryRow {
            id,
            path,
            file_name,
            note_total,
            instruments: Vec::new(),
        });
    }

    // 批量取乐器明细（分批 IN，避免超出 SQLite 变量上限）
    if !file_ids.is_empty() {
        const CHUNK: usize = 500;
        let mut map: HashMap<i64, Vec<(String, i64)>> = HashMap::new();
        for chunk in file_ids.chunks(CHUNK) {
            let ph = placeholders(chunk.len());
            let sql2 = format!(
                "SELECT fi.file_id, i.name, fi.note_count
                 FROM file_instruments fi
                 JOIN instruments i ON i.id = fi.instrument_id
                 WHERE fi.file_id IN ({ph})
                 ORDER BY fi.file_id, i.name"
            );
            let mut stmt2 = db.conn().prepare(&sql2)?;
            let sql_args2: Vec<&dyn rusqlite::ToSql> = chunk.iter().map(|id| {
                let v: &dyn rusqlite::ToSql = id;
                v
            }).collect();
            let rows2 = stmt2.query_map(rusqlite::params_from_iter(sql_args2), |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?;
            for r in rows2 {
                let (fid, name, cnt) = r?;
                map.entry(fid).or_default().push((name, cnt));
            }
        }
        for row in &mut out {
            row.instruments = map.remove(&row.id).unwrap_or_default();
        }
    }

    Ok(out)
}

fn resolve_instrument_db_id(db: &Repository, inst: &InstrumentId) -> Result<Option<i64>, CoreError> {
    let key = inst.db_key();
    let mut stmt = db.conn().prepare(
        "SELECT id FROM instruments WHERE bank_msb=?1 AND bank_lsb=?2 AND program=?3 AND is_percussion=?4",
    )?;
    let mut rows = stmt.query_map(
        rusqlite::params![key.0, key.1, key.2, key.3 as i64],
        |row| row.get::<_, i64>(0),
    )?;
    Ok(rows.next().transpose()?)
}

fn placeholders(n: usize) -> String {
    std::iter::repeat("?")
        .take(n)
        .collect::<Vec<_>>()
        .join(",")
}
