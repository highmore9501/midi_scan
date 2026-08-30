//! MIDI 解析层：自包含实现，解析核心借鉴 `key_ripple_rust::midi::midi_processor`
//! 的写法（midly 加载 + GM 128 名表 + NoteOn(vel>0) 口径），不修改、不依赖原仓库。

pub mod gm;
pub mod model;
pub mod parser;

pub use model::{InstrumentId, InstrumentStat, MidiFileInfo};
pub use parser::{extract_file_info, ParseError};
