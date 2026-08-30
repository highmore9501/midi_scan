//! 数据模型：统计维度 = 乐器 × 音符数量（D3）

use crate::gm;

/// 乐器标识（DB 主键与指纹依据）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct InstrumentId {
    pub bank_msb: u8,        // v1 恒为 0（GM）
    pub bank_lsb: u8,        // v1 恒为 0
    pub program: u8,         // 常规乐器 = ProgramChange 0..=127；打击乐 = NoteOn 音符号
    pub is_percussion: bool, // 通道 10（midly index 9）恒为 true（D2）
}

impl InstrumentId {
    pub fn db_key(&self) -> (i64, i64, i64, bool) {
        (
            self.bank_msb as i64,
            self.bank_lsb as i64,
            self.program as i64,
            self.is_percussion,
        )
    }

    pub fn from_db_key(key: (i64, i64, i64, bool)) -> Self {
        Self {
            bank_msb: key.0 as u8,
            bank_lsb: key.1 as u8,
            program: key.2 as u8,
            is_percussion: key.3,
        }
    }

    /// 显示名：常规乐器用 GM 名；打击乐用 GM 打击乐名（未知则回退编号）
    pub fn display_name(&self) -> String {
        if self.is_percussion {
            let name = gm::percussion_name(self.program);
            if name.is_empty() {
                format!("打击乐 {}", self.program)
            } else {
                name.to_string()
            }
        } else {
            gm::gm_name(self.program).to_string()
        }
    }
}

/// 乐器统计（统计维度 = 乐器 × 音符数量，而非轨道）
#[derive(Debug, Clone)]
pub struct InstrumentStat {
    pub instrument: InstrumentId,
    pub note_count: u64,          // 该乐器名下的音符总数（NoteOn vel>0）
    pub channels: Vec<u8>,        // 出现该乐器的通道（midly 0-based index，人类通道 = index+1）
    pub track_indexes: Vec<u16>,  // 出现该乐器的轨道
}

/// 单文件解析结果
#[derive(Debug, Clone, Default)]
pub struct MidiFileInfo {
    pub format: u16,
    pub track_count: u16,
    pub instruments: Vec<InstrumentStat>, // 按乐器聚合，跨轨道/通道合并
}
