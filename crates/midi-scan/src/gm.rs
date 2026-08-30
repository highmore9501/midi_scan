//! GM 音色名表：GM 128 常规音色（复制自 key_ripple_rust 的 midi_instruments 表）
//! + GM 打击乐名表（GM 标准，音符号 27..=87）。

/// GM 128 常规音色名（索引 = program 号）
pub const GM_NAMES: [&str; 128] = [
    "Acoustic Grand Piano",
    "Bright Acoustic Piano",
    "Electric Grand Piano",
    "Honky-tonk Piano",
    "Electric Piano 1",
    "Electric Piano 2",
    "Harpsichord",
    "Clavi",
    "Celesta",
    "Glockenspiel",
    "Music Box",
    "Vibraphone",
    "Marimba",
    "Xylophone",
    "Tubular Bells",
    "Dulcimer",
    "Drawbar Organ",
    "Percussive Organ",
    "Rock Organ",
    "Church Organ",
    "Reed Organ",
    "Accordion",
    "Harmonica",
    "Tango Accordion",
    "Acoustic Guitar (nylon)",
    "Acoustic Guitar (steel)",
    "Electric Guitar (jazz)",
    "Electric Guitar (clean)",
    "Electric Guitar (muted)",
    "Overdriven Guitar",
    "Distortion Guitar",
    "Guitar harmonics",
    "Acoustic Bass",
    "Electric Bass (finger)",
    "Electric Bass (pick)",
    "Fretless Bass",
    "Slap Bass 1",
    "Slap Bass 2",
    "Synth Bass 1",
    "Synth Bass 2",
    "Violin",
    "Viola",
    "Cello",
    "Contrabass",
    "Tremolo Strings",
    "Pizzicato Strings",
    "Orchestral Harp",
    "Timpani",
    "String Ensemble 1",
    "String Ensemble 2",
    "SynthStrings 1",
    "SynthStrings 2",
    "Choir Aahs",
    "Voice Oohs",
    "Synth Voice",
    "Orchestra Hit",
    "Trumpet",
    "Trombone",
    "Tuba",
    "Muted Trumpet",
    "French Horn",
    "Brass Section",
    "SynthBrass 1",
    "SynthBrass 2",
    "Soprano Sax",
    "Alto Sax",
    "Tenor Sax",
    "Baritone Sax",
    "Oboe",
    "English Horn",
    "Bassoon",
    "Clarinet",
    "Piccolo",
    "Flute",
    "Recorder",
    "Pan Flute",
    "Blown Bottle",
    "Shakuhachi",
    "Whistle",
    "Ocarina",
    "Lead 1 (square)",
    "Lead 2 (sawtooth)",
    "Lead 3 (calliope)",
    "Lead 4 (chiff)",
    "Lead 5 (charang)",
    "Lead 6 (voice)",
    "Lead 7 (fifths)",
    "Lead 8 (bass + lead)",
    "Pad 1 (new age)",
    "Pad 2 (warm)",
    "Pad 3 (polysynth)",
    "Pad 4 (choir)",
    "Pad 5 (bowed)",
    "Pad 6 (metallic)",
    "Pad 7 (halo)",
    "Pad 8 (sweep)",
    "FX 1 (rain)",
    "FX 2 (soundtrack)",
    "FX 3 (crystal)",
    "FX 4 (atmosphere)",
    "FX 5 (brightness)",
    "FX 6 (goblins)",
    "FX 7 (echoes)",
    "FX 8 (sci-fi)",
    "Sitar",
    "Banjo",
    "Shamisen",
    "Koto",
    "Kalimba",
    "Bag pipe",
    "Fiddle",
    "Shanai",
    "Tinkle Bell",
    "Agogo",
    "Steel Drums",
    "Woodblock",
    "Taiko Drum",
    "Melodic Tom",
    "Synth Drum",
    "Reverse Cymbal",
    "Guitar Fret Noise",
    "Breath Noise",
    "Seashore",
    "Bird Tweet",
    "Telephone Ring",
    "Helicopter",
    "Applause",
    "Gunshot",
];

/// GM 打击乐名表（索引 = 音符号；未定义的名称为空字符串）
pub const GM_PERCUSSION: [&str; 128] = {
    let mut t = [""; 128];
    t[27] = "High Q";
    t[28] = "Slap";
    t[29] = "Scratch Push";
    t[30] = "Scratch Pull";
    t[31] = "Sticks";
    t[32] = "Square Click";
    t[33] = "Metronome Click";
    t[34] = "Metronome Bell";
    t[35] = "Acoustic Bass Drum";
    t[36] = "Bass Drum 1";
    t[37] = "Side Stick";
    t[38] = "Acoustic Snare";
    t[39] = "Hand Clap";
    t[40] = "Electric Snare";
    t[41] = "Low Floor Tom";
    t[42] = "Closed Hi-Hat";
    t[43] = "High Floor Tom";
    t[44] = "Pedal Hi-Hat";
    t[45] = "Low Tom";
    t[46] = "Open Hi-Hat";
    t[47] = "Low-Mid Tom";
    t[48] = "Hi-Mid Tom";
    t[49] = "Crash Cymbal 1";
    t[50] = "High Tom";
    t[51] = "Ride Cymbal 1";
    t[52] = "Chinese Cymbal";
    t[53] = "Ride Bell";
    t[54] = "Tambourine";
    t[55] = "Splash Cymbal";
    t[56] = "Cowbell";
    t[57] = "Crash Cymbal 2";
    t[58] = "Vibraslap";
    t[59] = "Ride Cymbal 2";
    t[60] = "Hi Bongo";
    t[61] = "Low Bongo";
    t[62] = "Mute Hi Conga";
    t[63] = "Open Hi Conga";
    t[64] = "Low Conga";
    t[65] = "High Timbale";
    t[66] = "Low Timbale";
    t[67] = "High Agogo";
    t[68] = "Low Agogo";
    t[69] = "Cabasa";
    t[70] = "Maracas";
    t[71] = "Short Whistle";
    t[72] = "Long Whistle";
    t[73] = "Short Guiro";
    t[74] = "Long Guiro";
    t[75] = "Claves";
    t[76] = "Hi Wood Block";
    t[77] = "Low Wood Block";
    t[78] = "Mute Cuica";
    t[79] = "Open Cuica";
    t[80] = "Mute Triangle";
    t[81] = "Open Triangle";
    t[82] = "Shaker";
    t[83] = "Jingle Bell";
    t[84] = "Belltree";
    t[85] = "Castanets";
    t[86] = "Mute Surdo";
    t[87] = "Open Surdo";
    t
};

/// 常规乐器显示名（program 越界时回退 "Unknown instrument"）
pub fn gm_name(program: u8) -> &'static str {
    GM_NAMES.get(program as usize).copied().unwrap_or("Unknown instrument")
}

/// 打击乐显示名（未定义时返回空字符串，由调用方回退为「打击乐 N」）
pub fn percussion_name(note: u8) -> &'static str {
    GM_PERCUSSION.get(note as usize).copied().unwrap_or("")
}
