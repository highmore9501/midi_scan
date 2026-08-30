//! 解析核心：借鉴 key_ripple_rust `midi_processor.rs` 的写法
//! （`Smf::parse` 加载、NoteOn(vel>0) 口径），在本 crate 内自包含实现；
//! 与 key_ripple_rust 仓库无任何依赖关系。

use std::collections::{BTreeMap, HashMap};
use std::path::Path;

use midly::{Smf, TrackEventKind};

use crate::model::{InstrumentId, InstrumentStat, MidiFileInfo};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("无法读取文件: {0}")]
    Io(#[from] std::io::Error),
    #[error("MIDI 解析失败: {0}")]
    Smf(String),
}

/// 解析一个 MIDI 文件，按乐器聚合音符统计（D3）
pub fn extract_file_info(path: &Path) -> Result<MidiFileInfo, ParseError> {
    let data = std::fs::read(path)?;
    let smf = Smf::parse(&data).map_err(|e| ParseError::Smf(e.to_string()))?;

    let format = match smf.header.format {
        midly::Format::SingleTrack => 0u16,
        midly::Format::Parallel => 1u16,
        midly::Format::Sequential => 2u16,
    };
    let track_count = smf.tracks.len() as u16;

    let stats = collect_instrument_stats(&smf);
    let mut instruments: Vec<InstrumentStat> = stats.into_values().collect();
    instruments.sort_by_key(|s| s.instrument);

    Ok(MidiFileInfo {
        format,
        track_count,
        instruments,
    })
}

/// 逐音符归属当前生效 program，按乐器聚合（D1/D2/D3）：
/// - 常规乐器：归属该 (track, channel) 当前生效的 ProgramChange（无则默认 0，D8）；
/// - 通道 10（midly 0-based index == 9）：恒为打击乐，以 NoteOn 音符号为乐器标识（D2）；
/// - NoteOn(vel=0) 视为 NoteOff，不计数（D5）。
fn collect_instrument_stats(smf: &Smf) -> BTreeMap<(u8, u8, u8, bool), InstrumentStat> {
    // 每个 (track, channel) 当前生效的 program
    let mut current_program: HashMap<(usize, u8), u8> = HashMap::new();
    // 乐器 key(bank_msb, bank_lsb, program, is_percussion) → 统计（BTreeMap 保证输出顺序确定）
    let mut stats: BTreeMap<(u8, u8, u8, bool), InstrumentStat> = BTreeMap::new();

    for (track_index, track) in smf.tracks.iter().enumerate() {
        for event in track {
            if let TrackEventKind::Midi { channel, message } = event.kind {
                let channel_num = channel.as_int();
                match message {
                    midly::MidiMessage::ProgramChange { program } => {
                        // 打击乐通道不参与 program 切换
                        if channel_num != 9 {
                            current_program.insert((track_index, channel_num), program.as_int());
                        }
                    }
                    midly::MidiMessage::NoteOn { key, vel } => {
                        if vel.as_int() == 0 {
                            continue; // NoteOn(vel=0) 视为 NoteOff，不计数（D5）
                        }
                        let is_percussion = channel_num == 9;
                        let program = if is_percussion {
                            key.as_int()
                        } else {
                            // D8：无 ProgramChange 默认 program 0（GM Acoustic Grand Piano）
                            *current_program.get(&(track_index, channel_num)).unwrap_or(&0)
                        };
                        let entry = stats
                            .entry((0, 0, program, is_percussion))
                            .or_insert_with(|| InstrumentStat {
                                instrument: InstrumentId {
                                    bank_msb: 0,
                                    bank_lsb: 0,
                                    program,
                                    is_percussion,
                                },
                                note_count: 0,
                                channels: Vec::new(),
                                track_indexes: Vec::new(),
                            });
                        entry.note_count += 1;
                        if !entry.channels.contains(&channel_num) {
                            entry.channels.push(channel_num);
                        }
                        if !entry.track_indexes.contains(&(track_index as u16)) {
                            entry.track_indexes.push(track_index as u16);
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    stats
}
