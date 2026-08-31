//! midi-mgr 命令行入口：scan / query / dedup / db / tui

use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use midi_scan::InstrumentId;
use mm_core::duplicate::DupGroup;
use mm_core::query::{QueryParams, QueryRow};
use mm_core::service::Service;

#[derive(Parser)]
#[command(name = "midi-mgr", version, about = "MIDI 文件扫描与管理工具")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// 扫描目录中的 MIDI 文件并入库
    Scan {
        /// 要扫描的目录（可多次指定）
        #[arg(long = "dir", value_name = "DIR", required = true)]
        dirs: Vec<PathBuf>,
        #[arg(long)]
        db: Option<PathBuf>,
        /// 扫描后不触发全库去重检测
        #[arg(long)]
        no_dedup: bool,
        /// 扫描期自动去重：off（默认，进候选组）/ byte（字节相同直接删，v0.11 起仅支持字节相同）
        #[arg(long, value_name = "MODE", default_value = "off")]
        auto_dedup: String,
    },
    /// 组合查询（Exact 默认；--superset 切换为「包含所选全部乐器，允许更多」）
    Query {
        /// 乐器名（逗号分隔；支持 GM 名 / 打击乐名 / program:N / perc:N）
        #[arg(long)]
        instruments: Option<String>,
        /// 包含所选全部乐器（允许包含其他乐器），默认精确匹配
        #[arg(long)]
        superset: bool,
        /// 所选乐器各自的音符数下限（需配合 --instruments）
        #[arg(long)]
        note_min: Option<u64>,
        /// 所选乐器各自的音符数上限（需配合 --instruments）
        #[arg(long)]
        note_max: Option<u64>,
        /// 文件总音符数下限
        #[arg(long)]
        total_min: Option<u64>,
        /// 文件总音符数上限
        #[arg(long)]
        total_max: Option<u64>,
        /// 文件名关键词
        #[arg(long)]
        name: Option<String>,
        /// 限定目录前缀
        #[arg(long)]
        dir: Option<String>,
        #[arg(long)]
        db: Option<PathBuf>,
        /// 以 JSON 输出
        #[arg(long)]
        json: bool,
        #[arg(long, default_value_t = 50)]
        limit: u64,
        #[arg(long, default_value_t = 0)]
        offset: u64,
    },
    /// 去重中心：查看 / 确认删除重复文件
    Dedup {
        /// 只列出候选组，不删除
        #[arg(long)]
        dry_run: bool,
        /// 逐组交互确认（默认行为）
        #[arg(long)]
        interactive: bool,
        /// 自动处理全部候选组：oldest / newest / shortest
        #[arg(long, value_name = "RULE")]
        keep: Option<String>,
        #[arg(long)]
        db: Option<PathBuf>,
    },
    /// 数据库信息与统计
    Db {
        #[command(subcommand)]
        cmd: DbCmd,
        #[arg(long)]
        db: Option<PathBuf>,
    },
}

#[derive(Subcommand)]
enum DbCmd {
    /// 库内文件状态统计
    Info,
    /// 乐器分布等统计
    Stats,
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::WARN)
        .init();
    let cli = Cli::parse();
    match cli.command {
        Command::Scan {
            dirs,
            db,
            no_dedup,
            auto_dedup,
        } => cmd_scan(dirs, db, no_dedup, auto_dedup),
        Command::Query {
            instruments,
            superset,
            note_min,
            note_max,
            total_min,
            total_max,
            name,
            dir,
            db,
            json,
            limit,
            offset,
        } => cmd_query(
            instruments,
            superset,
            note_min,
            note_max,
            total_min,
            total_max,
            name,
            dir,
            db,
            json,
            limit,
            offset,
        ),
        Command::Dedup {
            dry_run,
            interactive,
            keep,
            db,
        } => cmd_dedup(dry_run, interactive, keep, db),
        Command::Db { cmd, db } => cmd_db(cmd, db),
    }
}

fn default_db_path() -> PathBuf {
    let base = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .unwrap_or_else(|_| ".".to_string());
    PathBuf::from(base)
        .join(".midi-manager")
        .join("library.sqlite")
}

// ---------- scan ----------

fn cmd_scan(
    dirs: Vec<PathBuf>,
    db: Option<PathBuf>,
    no_dedup: bool,
    auto_dedup: String,
) -> Result<()> {
    let db_path = db.unwrap_or_else(default_db_path);
    let mut svc = Service::open(&db_path).context("打开数据库失败")?;
    let mode = match auto_dedup.as_str() {
        "byte" => mm_core::scan::AutoDedupMode::Byte,
        _ => mm_core::scan::AutoDedupMode::Off,
    };
    let s = svc.scan(&dirs, !no_dedup, mode, None, None)?;
    println!(
        "扫描完成：发现 {}，新增 {}，更新 {}，跳过 {}，失败 {}，自动删除重复 {}，missing {}，去重候选 {}",
        s.found,
        s.new,
        s.updated,
        s.skipped,
        s.failed,
        s.deleted_duplicates,
        s.missing,
        s.duplicate_candidates
    );
    println!("数据库: {}", db_path.display());
    Ok(())
}

// ---------- query ----------

fn cmd_query(
    instruments: Option<String>,
    superset: bool,
    note_min: Option<u64>,
    note_max: Option<u64>,
    total_min: Option<u64>,
    total_max: Option<u64>,
    name: Option<String>,
    dir: Option<String>,
    db: Option<PathBuf>,
    json: bool,
    limit: u64,
    offset: u64,
) -> Result<()> {
    let db_path = db.unwrap_or_else(default_db_path);
    let svc = Service::open(&db_path).context("打开数据库失败")?;

    let selected = match &instruments {
        Some(s) => parse_instruments(s)?,
        None => Vec::new(),
    };
    let mut per_instrument_note_range = HashMap::new();
    if !selected.is_empty() {
        if let (Some(lo), Some(hi)) = (note_min, note_max) {
            if lo > hi {
                bail!("note_min 不能大于 note_max");
            }
            for inst in &selected {
                per_instrument_note_range.insert(*inst, (lo, hi));
            }
        } else if note_min.is_some() || note_max.is_some() {
            bail!("--note-min / --note-max 需要同时给出，并配合 --instruments 使用");
        }
    } else if note_min.is_some() || note_max.is_some() {
        bail!("--note-min / --note-max 需要配合 --instruments 使用");
    }

    let total_range = match (total_min, total_max) {
        (None, None) => None,
        (lo, hi) => Some((lo.unwrap_or(0), hi.unwrap_or(u64::MAX))),
    };

    let params = QueryParams {
        selected,
        match_mode: if superset {
            mm_core::query::MatchMode::Superset
        } else {
            mm_core::query::MatchMode::Exact
        },
        per_instrument_note_range,
        total_note_range: total_range,
        name_keyword: name,
        dir_filter: dir,
        limit,
        offset,
    };
    let rows = svc.query(&params)?;

    if json {
        let arr: Vec<serde_json::Value> = rows.iter().map(row_to_json).collect();
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::Value::Array(arr))?
        );
    } else {
        for r in &rows {
            let insts: Vec<String> = r
                .instruments
                .iter()
                .map(|(n, c)| format!("{n}:{c}"))
                .collect();
            println!(
                "{} | {} | 总音符 {} | {}",
                r.file_name,
                r.path,
                r.note_total,
                insts.join(", ")
            );
        }
        println!("共 {} 条", rows.len());
    }
    Ok(())
}

fn row_to_json(r: &QueryRow) -> serde_json::Value {
    serde_json::json!({
        "file_name": r.file_name,
        "path": r.path,
        "note_total": r.note_total,
        "instruments": r.instruments.iter().map(|(n, c)| serde_json::json!({"name": n, "note_count": c})).collect::<Vec<_>>(),
    })
}

fn parse_instruments(s: &str) -> Result<Vec<InstrumentId>> {
    let mut out = Vec::new();
    for token in s.split(',') {
        let t = token.trim();
        if t.is_empty() {
            continue;
        }
        out.push(parse_instrument(t)?);
    }
    if out.is_empty() {
        bail!("未解析到任何乐器");
    }
    Ok(out)
}

fn parse_instrument(t: &str) -> Result<InstrumentId> {
    if let Some(n) = t.strip_prefix("program:") {
        let p: u8 = n.trim().parse().context("program 号无效")?;
        if p > 127 {
            bail!("program 号必须在 0..=127");
        }
        return Ok(InstrumentId {
            bank_msb: 0,
            bank_lsb: 0,
            program: p,
            is_percussion: false,
        });
    }
    if let Some(n) = t.strip_prefix("perc:") {
        let p: u8 = n.trim().parse().context("打击乐音符号无效")?;
        return Ok(InstrumentId {
            bank_msb: 0,
            bank_lsb: 0,
            program: p,
            is_percussion: true,
        });
    }
    for (i, name) in midi_scan::gm::GM_NAMES.iter().enumerate() {
        if name.eq_ignore_ascii_case(t) {
            return Ok(InstrumentId {
                bank_msb: 0,
                bank_lsb: 0,
                program: i as u8,
                is_percussion: false,
            });
        }
    }
    for (i, name) in midi_scan::gm::GM_PERCUSSION.iter().enumerate() {
        if !name.is_empty() && name.eq_ignore_ascii_case(t) {
            return Ok(InstrumentId {
                bank_msb: 0,
                bank_lsb: 0,
                program: i as u8,
                is_percussion: true,
            });
        }
    }
    bail!("未知乐器: {t}（支持 GM 名、打击乐名、program:N、perc:N）")
}

// ---------- dedup ----------

fn cmd_dedup(
    dry_run: bool,
    interactive: bool,
    keep: Option<String>,
    db: Option<PathBuf>,
) -> Result<()> {
    let db_path = db.unwrap_or_else(default_db_path);
    let mut svc = Service::open(&db_path).context("打开数据库失败")?;
    let detected = svc.detect_duplicates()?;
    if detected.processed_groups > 0 || detected.processed_files > 0 {
        let remain = if detected.remaining_groups > 0 {
            format!(
                "；还有 {} 组待检测（处理完当前批次后可再次运行）",
                detected.remaining_groups
            )
        } else {
            String::new()
        };
        println!(
            "重新检测：{} 个候选组，{} 个候选文件{}",
            detected.processed_groups, detected.processed_files, remain
        );
    } else if detected.remaining_groups > 0 {
        println!(
            "还有 {} 组重复待检测（当前批次已满，处理完现有候选组后可再次运行）",
            detected.remaining_groups
        );
    }
    let groups = svc.pending_groups()?;
    if groups.is_empty() {
        println!("没有待处理的去重候选组");
        return Ok(());
    }
    if dry_run {
        for g in &groups {
            println!(
                "[组 {}] 类型 {} 指纹 {} 成员 {} 个:",
                g.id, g.dup_type, g.fingerprint, g.member_count
            );
            for m in &g.members {
                println!(
                    "  #{} {}（{} 音符，{} 字节，状态 {}）",
                    m.id, m.path, m.note_total, m.size_bytes, m.status
                );
            }
        }
        return Ok(());
    }
    if let Some(rule) = keep {
        return auto_resolve(&mut svc, &groups, &rule);
    }
    let _ = interactive; // 默认即为交互模式
    for g in &groups {
        interactive_group(&mut svc, g)?;
    }
    Ok(())
}

fn auto_resolve(svc: &mut Service, groups: &[DupGroup], rule: &str) -> Result<()> {
    for g in groups {
        let keep_id = match rule {
            "oldest" => g.members.first().map(|m| m.id),
            "newest" => g.members.last().map(|m| m.id),
            "shortest" => g.members.iter().min_by_key(|m| m.path.len()).map(|m| m.id),
            _ => bail!("--keep 支持 oldest / newest / shortest"),
        };
        let keep_id = keep_id.context("组内无成员")?;
        let deletes: Vec<i64> = g
            .members
            .iter()
            .map(|m| m.id)
            .filter(|id| *id != keep_id)
            .collect();
        let outcome = svc.resolve_group(g.id, keep_id, &deletes)?;
        println!(
            "组 {} 已处理：保留 #{}，删除 {} 个文件",
            g.id, keep_id, outcome.deleted
        );
    }
    Ok(())
}

fn interactive_group(svc: &mut Service, g: &DupGroup) -> Result<()> {
    println!(
        "\n===== 去重组 {}（{}）=====",
        g.id,
        if g.dup_type == "byte_identical" {
            "字节相同"
        } else {
            "结构相同"
        }
    );
    for (i, m) in g.members.iter().enumerate() {
        println!(
            "  [{}] #{} {}（{} 音符，{} 字节，首次入库 {}）",
            i, m.id, m.path, m.note_total, m.size_bytes, m.first_scanned_at
        );
    }
    let keep_line = read_line(&format!(
        "保留哪个？（默认 0 = 最早入库 #{}）: ",
        g.members[0].id
    ))?;
    let keep_idx: usize = if keep_line.trim().is_empty() {
        0
    } else {
        keep_line.trim().parse().context("输入无效")?
    };
    if keep_idx >= g.members.len() {
        bail!("索引越界");
    }
    let keep_id = g.members[keep_idx].id;

    let del_line = read_line("要删除的序号（逗号分隔，输入 all 全选删除其余）: ")?;
    let del_line = del_line.trim();
    let mut deletes = Vec::new();
    if del_line.eq_ignore_ascii_case("all") {
        deletes = g
            .members
            .iter()
            .map(|m| m.id)
            .filter(|id| *id != keep_id)
            .collect();
    } else if !del_line.is_empty() {
        for tok in del_line.split(',') {
            let idx: usize = tok.trim().parse().context("序号无效")?;
            if idx >= g.members.len() {
                bail!("序号越界");
            }
            let id = g.members[idx].id;
            if id != keep_id && !deletes.contains(&id) {
                deletes.push(id);
            }
        }
    }
    if deletes.is_empty() {
        println!("未选择任何文件，跳过该组");
        return Ok(());
    }
    println!("将【永久删除】以下 {} 个文件：", deletes.len());
    for id in &deletes {
        let m = g.members.iter().find(|m| m.id == *id).expect("成员已校验");
        println!("  {}", m.path);
    }
    let confirm = read_line("确认硬删？输入 yes 确认: ")?;
    if confirm.trim().eq_ignore_ascii_case("yes") {
        let outcome = svc.resolve_group(g.id, keep_id, &deletes)?;
        println!("已删除 {} 个文件，组 {} 已解决", outcome.deleted, g.id);
    } else {
        println!("已取消");
    }
    Ok(())
}

fn read_line(prompt: &str) -> Result<String> {
    use std::io::Write;
    print!("{prompt}");
    std::io::stdout().flush()?;
    let mut line = String::new();
    std::io::stdin().read_line(&mut line)?;
    Ok(line)
}

// ---------- db ----------

fn cmd_db(cmd: DbCmd, db: Option<PathBuf>) -> Result<()> {
    let db_path = db.unwrap_or_else(default_db_path);
    let svc = Service::open(&db_path).context("打开数据库失败")?;
    match cmd {
        DbCmd::Info => {
            println!("数据库: {}", db_path.display());
            for (status, count) in svc.db.counts_by_status()? {
                println!("  {status}: {count}");
            }
        }
        DbCmd::Stats => {
            println!("乐器分布 Top 20（按文件数）:");
            for (name, cnt) in svc.db.instrument_top(20)? {
                println!("  {name}: {cnt} 个文件");
            }
        }
    }
    Ok(())
}
